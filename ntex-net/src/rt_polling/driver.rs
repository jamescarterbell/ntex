use std::{cell::Cell, collections::VecDeque, fmt, io, ptr, rc::Rc, task, task::Poll};

use ntex_neon::driver::op::{CloseSocket, Handler, Interest};
use ntex_neon::driver::{AsRawFd, DriverApi, RawFd};
use ntex_neon::{syscall, Runtime};
use slab::Slab;

use ntex_bytes::BufMut;
use ntex_io::IoContext;

bitflags::bitflags! {
    #[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    struct Flags: u8 {
        const ERROR = 0b0000_0001;
        const RD    = 0b0000_0010;
        const WR    = 0b0000_0100;
    }
}

pub(crate) struct StreamCtl<T> {
    id: usize,
    inner: Rc<StreamOpsInner<T>>,
}

struct StreamItem<T> {
    io: Option<T>,
    fd: RawFd,
    context: IoContext,
    flags: Flags,
    ref_count: usize,
}

pub(crate) struct StreamOps<T>(Rc<StreamOpsInner<T>>);

#[derive(Debug)]
enum Change {
    Readable,
    Writable,
    Error(io::Error),
}

struct StreamOpsHandler<T> {
    feed: VecDeque<(usize, Change)>,
    inner: Rc<StreamOpsInner<T>>,
}

struct StreamOpsInner<T> {
    api: DriverApi,
    feed: Cell<Option<VecDeque<usize>>>,
    streams: Cell<Option<Box<Slab<StreamItem<T>>>>>,
}

impl<T: AsRawFd + 'static> StreamOps<T> {
    pub(crate) fn current() -> Self {
        Runtime::with_current(|rt| {
            if let Some(s) = rt.get::<Self>() {
                s
            } else {
                let mut inner = None;
                rt.driver().register_handler(|api| {
                    let ops = Rc::new(StreamOpsInner {
                        api,
                        feed: Cell::new(Some(VecDeque::new())),
                        streams: Cell::new(Some(Box::new(Slab::new()))),
                    });
                    inner = Some(ops.clone());
                    Box::new(StreamOpsHandler {
                        inner: ops,
                        feed: VecDeque::new(),
                    })
                });

                let s = StreamOps(inner.unwrap());
                rt.insert(s.clone());
                s
            }
        })
    }

    pub(crate) fn register(&self, io: T, context: IoContext) -> StreamCtl<T> {
        let item = StreamItem {
            context,
            fd: io.as_raw_fd(),
            io: Some(io),
            flags: Flags::empty(),
            ref_count: 1,
        };
        self.with(|streams| {
            let id = streams.insert(item);
            StreamCtl {
                id,
                inner: self.0.clone(),
            }
        })
    }

    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Slab<StreamItem<T>>) -> R,
    {
        let mut inner = self.0.streams.take().unwrap();
        let result = f(&mut inner);
        self.0.streams.set(Some(inner));
        result
    }
}

impl<T> Clone for StreamOps<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Handler for StreamOpsHandler<T> {
    fn readable(&mut self, id: usize) {
        log::debug!("FD is readable {:?}", id);
        self.feed.push_back((id, Change::Readable));
    }

    fn writable(&mut self, id: usize) {
        log::debug!("FD is writable {:?}", id);
        self.feed.push_back((id, Change::Writable));
    }

    fn error(&mut self, id: usize, err: io::Error) {
        log::debug!("FD is failed {:?}, err: {:?}", id, err);
        self.feed.push_back((id, Change::Error(err)));
    }

    fn commit(&mut self) {
        if self.feed.is_empty() {
            return;
        }
        log::debug!("Commit changes, num: {:?}", self.feed.len());

        let mut streams = self.inner.streams.take().unwrap();

        for (id, change) in self.feed.drain(..) {
            match change {
                Change::Readable => {
                    let item = &mut streams[id];
                    let result = item.context.with_read_buf(|buf| {
                        let chunk = buf.chunk_mut();
                        let b = chunk.as_mut_ptr();
                        Poll::Ready(
                            task::ready!(syscall!(
                                break libc::read(item.fd, b as _, chunk.len())
                            ))
                            .inspect(|size| {
                                unsafe { buf.advance_mut(*size) };
                                log::debug!(
                                    "{}: {:?}, SIZE: {:?}, BUF: {:?}",
                                    item.context.tag(),
                                    item.fd,
                                    size,
                                    buf
                                );
                            }),
                        )
                    });

                    if result.is_pending() {
                        item.flags.insert(Flags::RD);
                        self.inner.api.register(item.fd, id, Interest::Readable);
                    }
                }
                Change::Writable => {
                    let item = &mut streams[id];
                    let result = item.context.with_write_buf(|buf| {
                        let slice = &buf[..];
                        syscall!(
                            break libc::write(item.fd, slice.as_ptr() as _, slice.len())
                        )
                    });

                    if result.is_pending() {
                        item.flags.insert(Flags::WR);
                        self.inner.api.register(item.fd, id, Interest::Writable);
                    }
                }
                Change::Error(err) => {
                    if let Some(item) = streams.get_mut(id) {
                        item.context.stopped(Some(err));
                        if !item.flags.contains(Flags::ERROR) {
                            item.flags.insert(Flags::ERROR);
                            item.flags.remove(Flags::RD | Flags::WR);
                            self.inner.api.unregister_all(item.fd);
                        }
                    }
                }
            }
        }

        // extra
        let mut feed = self.inner.feed.take().unwrap();
        for id in feed.drain(..) {
            let item = &mut streams[id];
            log::debug!("{}: Drop io ({}), {:?}", item.context.tag(), id, item.fd);

            item.ref_count -= 1;
            if item.ref_count == 0 {
                let item = streams.remove(id);
                if item.io.is_some() {
                    self.inner.api.unregister_all(item.fd);
                }
            }
        }

        self.inner.feed.set(Some(feed));
        self.inner.streams.set(Some(streams));
    }
}

impl<T> StreamCtl<T> {
    pub(crate) async fn close(self) -> io::Result<()> {
        let (io, fd) =
            self.with(|streams| (streams[self.id].io.take(), streams[self.id].fd));
        if let Some(io) = io {
            let op = CloseSocket::from_raw_fd(fd);
            let fut = ntex_neon::submit(op);
            std::mem::forget(io);
            fut.await.0?;
        }
        Ok(())
    }

    pub(crate) fn with_io<F, R>(&self, f: F) -> R
    where
        F: FnOnce(Option<&T>) -> R,
    {
        self.with(|streams| f(streams[self.id].io.as_ref()))
    }

    pub(crate) fn pause_all(&self) {
        self.with(|streams| {
            let item = &mut streams[self.id];

            if item.flags.intersects(Flags::RD | Flags::WR) {
                log::debug!(
                    "{}: Pause all io ({}), {:?}",
                    item.context.tag(),
                    self.id,
                    item.fd
                );
                item.flags.remove(Flags::RD | Flags::WR);
                self.inner.api.unregister_all(item.fd);
            }
        })
    }

    pub(crate) fn pause_read(&self) {
        self.with(|streams| {
            let item = &mut streams[self.id];

            log::debug!(
                "{}: Pause io read ({}), {:?}",
                item.context.tag(),
                self.id,
                item.fd
            );
            if item.flags.contains(Flags::RD) {
                item.flags.remove(Flags::RD);
                self.inner.api.unregister(item.fd, Interest::Readable);
            }
        })
    }

    pub(crate) fn resume_read(&self) {
        self.with(|streams| {
            let item = &mut streams[self.id];

            log::debug!(
                "{}: Resume io read ({}), {:?}",
                item.context.tag(),
                self.id,
                item.fd
            );
            if !item.flags.contains(Flags::RD) {
                item.flags.insert(Flags::RD);
                self.inner
                    .api
                    .register(item.fd, self.id, Interest::Readable);
            }
        })
    }

    pub(crate) fn resume_write(&self) {
        self.with(|streams| {
            let item = &mut streams[self.id];

            if !item.flags.contains(Flags::WR) {
                log::debug!(
                    "{}: Resume io write ({}), {:?}",
                    item.context.tag(),
                    self.id,
                    item.fd
                );
                let result = item.context.with_write_buf(|buf| {
                    log::debug!(
                        "{}: Writing io ({}), buf: {:?}",
                        item.context.tag(),
                        self.id,
                        buf.len()
                    );

                    let slice = &buf[..];
                    syscall!(break libc::write(item.fd, slice.as_ptr() as _, slice.len()))
                });

                if result.is_pending() {
                    log::debug!(
                        "{}: Write is pending ({}), {:?}",
                        item.context.tag(),
                        self.id,
                        item.context.flags()
                    );

                    item.flags.insert(Flags::WR);
                    self.inner
                        .api
                        .register(item.fd, self.id, Interest::Writable);
                }
            }
        })
    }

    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Slab<StreamItem<T>>) -> R,
    {
        let mut inner = self.inner.streams.take().unwrap();
        let result = f(&mut inner);
        self.inner.streams.set(Some(inner));
        result
    }
}

impl<T> Clone for StreamCtl<T> {
    fn clone(&self) -> Self {
        self.with(|streams| {
            streams[self.id].ref_count += 1;
            Self {
                id: self.id,
                inner: self.inner.clone(),
            }
        })
    }
}

impl<T> Drop for StreamCtl<T> {
    fn drop(&mut self) {
        if let Some(mut streams) = self.inner.streams.take() {
            log::debug!(
                "{}: Drop io ({}), {:?}",
                streams[self.id].context.tag(),
                self.id,
                streams[self.id].fd
            );

            streams[self.id].ref_count -= 1;
            if streams[self.id].ref_count == 0 {
                let item = streams.remove(self.id);
                if item.io.is_some() {
                    self.inner.api.unregister_all(item.fd);
                }
            }
            self.inner.streams.set(Some(streams));
        } else {
            let mut feed = self.inner.feed.take().unwrap();
            feed.push_back(self.id);
            self.inner.feed.set(Some(feed));
        }
    }
}

impl<T> PartialEq for StreamCtl<T> {
    #[inline]
    fn eq(&self, other: &StreamCtl<T>) -> bool {
        self.id == other.id && ptr::eq(&self.inner, &other.inner)
    }
}

impl<T: fmt::Debug> fmt::Debug for StreamCtl<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.with(|streams| {
            f.debug_struct("StreamCtl")
                .field("id", &self.id)
                .field("io", &streams[self.id].io)
                .finish()
        })
    }
}
