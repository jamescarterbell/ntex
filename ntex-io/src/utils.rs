use std::{fmt, marker::PhantomData};

use ntex_service::{chain_factory, fn_service, Service, ServiceCtx, ServiceFactory};
use ntex_util::future::Ready;

use crate::{Filter, FilterFactory, Io, IoBoxed, Layer};

/// Decoded item from buffer
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Decoded<T> {
    pub item: Option<T>,
    pub remains: usize,
    pub consumed: usize,
}

/// Service that converts any Io<F> stream to IoBoxed stream
pub fn seal<F, S, C>(
    srv: S,
) -> impl ServiceFactory<
    Io<F>,
    C,
    Response = S::Response,
    Error = S::Error,
    InitError = S::InitError,
>
where
    F: Filter,
    S: ServiceFactory<IoBoxed, C>,
    C: Clone,
{
    chain_factory(fn_service(|io: Io<F>| Ready::Ok(IoBoxed::from(io))))
        .map_init_err(|_| panic!())
        .and_then(srv)
}

/// Create filter factory service
pub fn filter<T, F>(filter: T) -> FilterServiceFactory<T, F>
where
    T: FilterFactory<F> + Clone,
{
    FilterServiceFactory {
        filter,
        _t: PhantomData,
    }
}

pub struct FilterServiceFactory<T, F> {
    filter: T,
    _t: PhantomData<F>,
}

impl<T: FilterFactory<F> + fmt::Debug, F> fmt::Debug for FilterServiceFactory<T, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilterServiceFactory")
            .field("filter_factory", &self.filter)
            .finish()
    }
}

impl<T, F> ServiceFactory<Io<F>> for FilterServiceFactory<T, F>
where
    T: FilterFactory<F> + Clone,
{
    type Response = Io<Layer<T::Filter, F>>;
    type Error = T::Error;
    type Service = FilterService<T, F>;
    type InitError = ();

    async fn create(&self, _: ()) -> Result<Self::Service, Self::InitError> {
        Ok(FilterService {
            filter: self.filter.clone(),
            _t: PhantomData,
        })
    }
}

pub struct FilterService<T, F> {
    filter: T,
    _t: PhantomData<F>,
}

impl<T: FilterFactory<F> + fmt::Debug, F> fmt::Debug for FilterService<T, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilterService")
            .field("filter_factory", &self.filter)
            .finish()
    }
}

impl<T, F> Service<Io<F>> for FilterService<T, F>
where
    T: FilterFactory<F> + Clone,
{
    type Response = Io<Layer<T::Filter, F>>;
    type Error = T::Error;

    #[inline]
    async fn call(
        &self,
        req: Io<F>,
        _: ServiceCtx<'_, Self>,
    ) -> Result<Self::Response, Self::Error> {
        self.filter.clone().create(req).await
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use ntex_bytes::Bytes;
    use ntex_codec::BytesCodec;

    use super::*;
    use crate::{
        buf::Stack, filter::NullFilter, testing::IoTest, FilterLayer, ReadBuf, WriteBuf,
    };

    #[ntex::test]
    async fn test_utils() {
        let (client, server) = IoTest::create();
        client.remote_buffer_cap(1024);
        client.write("REQ");

        let svc = seal(fn_service(|io: IoBoxed| async move {
            let t = io.recv(&BytesCodec).await.unwrap().unwrap();
            assert_eq!(t, b"REQ".as_ref());
            io.send(Bytes::from_static(b"RES"), &BytesCodec)
                .await
                .unwrap();
            Ok::<_, ()>(())
        }))
        .pipeline(())
        .await
        .unwrap();
        let _ = svc.call(Io::new(server)).await;

        let buf = client.read().await.unwrap();
        assert_eq!(buf, b"RES".as_ref());
    }

    #[derive(Debug)]
    pub(crate) struct TestFilter;

    impl FilterLayer for TestFilter {
        fn process_read_buf(&self, buf: &ReadBuf<'_>) -> io::Result<usize> {
            Ok(buf.nbytes())
        }

        fn process_write_buf(&self, _: &WriteBuf<'_>) -> io::Result<()> {
            Ok(())
        }
    }

    #[derive(Copy, Clone, Debug)]
    struct TestFilterFactory;

    impl<F: Filter> FilterFactory<F> for TestFilterFactory {
        type Filter = TestFilter;
        type Error = std::convert::Infallible;
        type Future = Ready<Io<Layer<TestFilter, F>>, Self::Error>;

        fn create(self, st: Io<F>) -> Self::Future {
            Ready::Ok(st.add_filter(TestFilter))
        }
    }

    #[ntex::test]
    async fn test_utils_filter() {
        let (_, server) = IoTest::create();

        let filter_service_factory = filter::<_, crate::filter::Base>(TestFilterFactory)
            .map_err(|_| ())
            .map_init_err(|_| ());

        assert!(format!("{:?}", filter_service_factory).contains("FilterServiceFactory"));

        let svc = chain_factory(filter_service_factory)
            .and_then(seal(fn_service(|io: IoBoxed| async move {
                let _ = io.recv(&BytesCodec).await;
                Ok::<_, ()>(())
            })))
            .pipeline(())
            .await
            .unwrap();

        let _ = svc.call(Io::new(server)).await;

        let (client, _) = IoTest::create();
        let io = Io::new(client);
        format!("{:?}", TestFilter);
        let mut s = Stack::new();
        s.add_layer();
        let _ = s.read_buf(&io, 0, 0, |b| TestFilter.process_read_buf(b));
        let _ = s.write_buf(&io, 0, |b| TestFilter.process_write_buf(b));
    }

    #[ntex::test]
    async fn test_null_filter() {
        let (_, server) = IoTest::create();
        let io = Io::new(server);
        let ioref = io.get_ref();
        let stack = Stack::new();
        assert!(NullFilter.query(std::any::TypeId::of::<()>()).is_none());
        assert!(NullFilter.shutdown(&ioref, &stack, 0).unwrap().is_ready());
        assert_eq!(
            std::future::poll_fn(|cx| NullFilter.poll_read_ready(cx)).await,
            crate::ReadStatus::Terminate
        );
        assert_eq!(
            std::future::poll_fn(|cx| NullFilter.poll_write_ready(cx)).await,
            crate::WriteStatus::Terminate
        );
        assert!(NullFilter.process_write_buf(&ioref, &stack, 0).is_ok());
        assert_eq!(
            NullFilter.process_read_buf(&ioref, &stack, 0, 0).unwrap(),
            Default::default()
        )
    }
}
