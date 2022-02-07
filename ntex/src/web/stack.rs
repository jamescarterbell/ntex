use std::{
    convert::Infallible, future::Future, marker::PhantomData, pin::Pin, rc::Rc,
    task::Context, task::Poll,
};

use crate::service::{
    dev::AndThenFactory, pipeline_factory, PipelineFactory, Service, ServiceFactory,
    Transform,
};
use crate::util::{ready, Ready};

use super::httprequest::{HttpRequest, WeakHttpRequest};
use super::{ErrorContainer, ErrorRenderer, WebRequest, WebResponse};

pub struct Stack<Inner, Outer, Err> {
    inner: Inner,
    outer: Outer,
    _t: PhantomData<Err>,
}

impl<Inner, Outer, Err> Stack<Inner, Outer, Err> {
    pub(super) fn new(inner: Inner, outer: Outer) -> Self {
        Stack {
            inner,
            outer,
            _t: PhantomData,
        }
    }
}

impl<S, Inner, Outer, Err> Transform<S> for Stack<Inner, Outer, Err>
where
    Err: ErrorRenderer,
    Inner: Transform<S>,
    Inner::Service: Service<WebRequest<Err>, Response = WebResponse>,
    <Inner::Service as Service<WebRequest<Err>>>::Error: Into<Err::Container>,
    Outer: Transform<Next<Inner::Service, Err>>,
{
    type Service = Outer::Service;

    fn new_transform(&self, service: S) -> Self::Service {
        self.outer
            .new_transform(Next::new(self.inner.new_transform(service)))
    }
}

pub(super) struct MiddlewareStack<T, Err> {
    inner: T,
    _t: PhantomData<Err>,
}

impl<T, Err> MiddlewareStack<T, Err> {
    pub(super) fn new(inner: T) -> Self {
        Self {
            inner,
            _t: PhantomData,
        }
    }
}

impl<S, T, Err> Transform<S> for MiddlewareStack<T, Err>
where
    T: Transform<S>,
{
    type Service = Middleware<T::Service, Err>;

    fn new_transform(&self, service: S) -> Self::Service {
        Middleware {
            md: self.inner.new_transform(service),
            _t: PhantomData,
        }
    }
}

pub struct Middleware<S, Err> {
    md: S,
    _t: PhantomData<Err>,
}

impl<'a, S, Err> Service<&'a mut WebRequest<Err>> for Middleware<S, Err>
where
    S: Service<&'a mut WebRequest<Err>, Response = WebResponse, Error = Infallible>,
    Err: ErrorRenderer,
{
    type Response = WebResponse;
    type Error = Err::Container;
    type Future = MiddlewareResponse<'a, S, Err>;

    #[inline]
    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let _ = ready!(self.md.poll_ready(cx));
        Poll::Ready(Ok(()))
    }

    #[inline]
    fn call(&self, req: &'a mut WebRequest<Err>) -> Self::Future {
        MiddlewareResponse {
            fut: self.md.call(req),
        }
    }
}

pin_project_lite::pin_project! {
    pub struct MiddlewareResponse<'a, S: Service<&'a mut WebRequest<Err>>, Err> {
        #[pin]
        fut: S::Future,
    }
}

impl<'a, S, Err> Future for MiddlewareResponse<'a, S, Err>
where
    S: Service<&'a mut WebRequest<Err>, Response = WebResponse, Error = Infallible>,
    Err: ErrorRenderer,
{
    type Output = Result<WebResponse, Err::Container>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(ready!(self.project().fut.poll(cx)).unwrap()))
    }
}

pub struct Next<S, Err> {
    inner: Rc<S>,
    _t: PhantomData<Err>,
}

impl<S, Err> Next<S, Err> {
    pub(super) fn new(inner: S) -> Self {
        Next {
            inner: Rc::new(inner),
            _t: PhantomData,
        }
    }
}

impl<'a, S, Err> Service<&'a mut WebRequest<Err>> for Next<S, Err>
where
    S: Service<&'a mut WebRequest<Err>, Response = WebResponse> + 'static,
    S::Error: Into<Err::Container> + 'static,
    S::Future: 'a,
    Err: ErrorRenderer,
{
    type Response = WebResponse;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + 'a>>;

    #[inline]
    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let _ = ready!(self.inner.poll_ready(cx));
        Poll::Ready(Ok(()))
    }

    #[inline]
    fn call(&self, req: &'a mut WebRequest<Err>) -> Self::Future {
        let next = self.inner.clone();
        Box::pin(async move {
            let result = next.call(req).await;
            match result {
                Ok(res) => Ok(res),
                Err(err) => Ok(WebResponse::new(err.into().error_response(&req.req))),
            }
        })
    }
}

pin_project_lite::pin_project! {
    pub struct NextResponse<'a, S: Service<&'a mut WebRequest<Err>>, Err> {
        #[pin]
        fut: S::Future,
        req: &'a mut WebRequest<Err>,
    }
}

impl<'a, S, Err> Future for NextResponse<'a, S, Err>
where
    S: Service<&'a mut WebRequest<Err>, Response = WebResponse>,
    S::Error: Into<Err::Container>,
    Err: ErrorRenderer,
{
    type Output = Result<WebResponse, Infallible>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match ready!(this.fut.poll(cx)) {
            Ok(res) => Poll::Ready(Ok(res)),
            Err(err) => {
                let req = this.req.req.clone();
                Poll::Ready(Ok(WebResponse::new(err.into().error_response(&req))))
            }
        }
    }
}

pub struct Filter<Err>(PhantomData<Err>);

impl<Err: ErrorRenderer> Filter<Err> {
    pub(super) fn new() -> Self {
        Filter(PhantomData)
    }
}

impl<'a, Err: ErrorRenderer> FiltersFactory<'a, Err> for Filter<Err> {
    type Service = Filter<Err>;

    fn create(self) -> Self::Service {
        self
    }
}

impl<'a, Err: ErrorRenderer> ServiceFactory<&'a mut WebRequest<Err>> for Filter<Err> {
    type Response = &'a mut WebRequest<Err>;
    type Error = Err::Container;
    type InitError = ();
    type Service = Filter<Err>;
    type Future = Ready<Filter<Err>, ()>;

    #[inline]
    fn new_service(&self, _: ()) -> Self::Future {
        Ready::Ok(Filter(PhantomData))
    }
}

impl<'a, Err: ErrorRenderer> Service<&'a mut WebRequest<Err>> for Filter<Err> {
    type Response = &'a mut WebRequest<Err>;
    type Error = Err::Container;
    type Future = Ready<Self::Response, Self::Error>;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[inline]
    fn call(&self, req: &'a mut WebRequest<Err>) -> Self::Future {
        Ready::Ok(req)
    }
}

pub struct Filters<First, Second, Err> {
    first: First,
    second: Second,
    _t: PhantomData<Err>,
}

impl<First, Second, Err> Filters<First, Second, Err> {
    pub(super) fn new(first: First, second: Second) -> Self {
        Filters {
            first,
            second,
            _t: PhantomData,
        }
    }
}

impl<'a, First, Second, Err> FiltersFactory<'a, Err> for Filters<First, Second, Err>
where
    Err: ErrorRenderer,
    First: ServiceFactory<
            &'a mut WebRequest<Err>,
            Response = &'a mut WebRequest<Err>,
            Error = Err::Container,
            InitError = (),
        > + 'static,
    First::Service: 'static,
    First::Future: 'static,
    Second: FiltersFactory<'a, Err>,
    Second::Service: ServiceFactory<
        &'a mut WebRequest<Err>,
        Response = &'a mut WebRequest<Err>,
        Error = Err::Container,
        InitError = (),
    >,
    <Second::Service as ServiceFactory<&'a mut WebRequest<Err>>>::Service: 'static,
    <Second::Service as ServiceFactory<&'a mut WebRequest<Err>>>::Future: 'static,
{
    type Service = AndThenFactory<First, Second::Service>;

    fn create(self) -> Self::Service {
        AndThenFactory::new(self.first, self.second.create())
    }
}

pub trait FiltersFactory<'a, Err: ErrorRenderer> {
    type Service: ServiceFactory<
            &'a mut WebRequest<Err>,
            Response = &'a mut WebRequest<Err>,
            Error = Err::Container,
            InitError = (),
        > + 'static;

    fn create(self) -> Self::Service;
}