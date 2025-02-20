//! See [`Service`] docs for information on this crate's foundational trait.
#![allow(async_fn_in_trait)]
#![deny(
    rust_2018_idioms,
    warnings,
    unreachable_pub,
    missing_debug_implementations
)]
use std::future::Future;
use std::{rc::Rc, task::Context};

mod and_then;
mod apply;
pub mod boxed;
// mod chain;
mod ctx;
mod fn_service;
mod fn_shutdown;
mod macros;
mod map;
mod map_config;
mod map_err;
mod map_init_err;
mod middleware;
mod pipeline;
mod then;
mod util;

// use chain::{chain_factory, chain, ServiceChain, ServiceChainFactory};
use dev::{AndThen, AndThenFactory, Apply, ApplyFactory, ApplyMiddleware, MapErr, MapErrFactory, MapInitErr, Then, ThenFactory};
use map::{Map, MapFactory};

pub use self::apply::{apply_fn, apply_fn_factory};
pub use self::ctx::ServiceCtx;
pub use self::fn_service::{fn_factory, fn_factory_with_config, fn_service};
pub use self::fn_shutdown::fn_shutdown;
pub use self::map_config::{map_config, unit_config};
pub use self::middleware::{apply, Identity, Middleware, Stack};
pub use self::pipeline::{Pipeline, PipelineBinding, PipelineCall};

#[allow(unused_variables)]
/// An asynchronous function of `Request` to a `Response`.
///
/// The `Service` trait represents a request/response interaction, receiving requests and returning
/// replies. You can think about service as a function with one argument that returns some result
/// asynchronously. Conceptually, the operation looks like this:
///
/// ```rust,ignore
/// async fn(Request) -> Result<Response, Error>
/// ```
///
/// The `Service` trait just generalizes this form. Requests are defined as a generic type parameter
/// and responses and other details are defined as associated types on the trait impl. Notice that
/// this design means that services can receive many request types and converge them to a single
/// response type.
///
/// Services can also have mutable state that influence computation by using a `Cell`, `RefCell`
/// or `Mutex`. Services intentionally do not take `&mut self` to reduce overhead in the
/// common cases.
///
/// `Service` provides a symmetric and uniform API; the same abstractions can be used to represent
/// both clients and servers. Services describe only _transformation_ operations which encourage
/// simple API surfaces. This leads to simpler design of each service, improves test-ability and
/// makes composition easier.
///
/// ```rust
/// # use std::convert::Infallible;
/// #
/// # use ntex_service::{Service, ServiceCtx};
///
/// struct MyService;
///
/// impl Service<u8> for MyService {
///     type Response = u64;
///     type Error = Infallible;
///
///     async fn call(&self, req: u8, ctx: ServiceCtx<'_, Self>) -> Result<Self::Response, Self::Error> {
///         Ok(req as u64)
///     }
/// }
/// ```
///
/// Sometimes it is not necessary to implement the Service trait. For example, the above service
/// could be rewritten as a simple function and passed to [`fn_service`](fn_service()).
///
/// ```rust,ignore
/// async fn my_service(req: u8) -> Result<u64, Infallible>;
/// ```
///
/// Service cannot be called directly, it must be wrapped to an instance of [`Pipeline``] or
/// by using `ctx` argument of the call method in case of chanined services.
///
pub trait Service<Req> {
    /// Responses given by the service.
    type Response;

    /// Errors produced by the service when polling readiness or executing call.
    type Error;

    /// Process the request and return the response asynchronously.
    ///
    /// This function is expected to be callable off-task. As such, implementations of `call`
    /// should take care to not call `poll_ready`. Caller of the service verifies readiness,
    /// Only way to make a `call` is to use `ctx` argument, it enforces readiness before calling
    /// service.
    async fn call(
        &self,
        req: Req,
        ctx: ServiceCtx<'_, Self>,
    ) -> Result<Self::Response, Self::Error>;

    #[inline]
    /// Returns when the service is able to process requests.
    ///
    /// If the service is at capacity, then `ready` does not returns and the task is notified when
    /// the service becomes ready again. This function is expected to be called while on a task.
    ///
    /// This is a **best effort** implementation. False positives are permitted. It is permitted for
    /// the service to returns from a `ready` call and the next invocation of `call`
    /// results in an error.
    async fn ready(&self, ctx: ServiceCtx<'_, Self>) -> Result<(), Self::Error> {
        Ok(())
    }

    #[deprecated]
    #[doc(hidden)]
    /// Returns when the service is not able to process requests.
    ///
    /// Unlike the "ready()" method, the "not_ready()" method returns
    /// only when the service becomes unready. This method is intended
    /// for actively monitoring and maintaining the service state.
    ///
    /// "not_ready()" implementation is optional.
    async fn not_ready(&self) {
        std::future::pending().await
    }

    #[inline]
    /// Shutdown service.
    ///
    /// Returns when the service is properly shutdowns.
    async fn shutdown(&self) {}

    #[inline]
    /// Polls service from the current task.
    ///
    /// Service may require to execute asynchronous computation or
    /// maintain asynchronous state.
    fn poll(&self, cx: &mut Context<'_>) -> Result<(), Self::Error> {
        Ok(())
    }

    // fn chain(self) -> ServiceChain<Self, Req> {
    //     chain(self)
    // }

    /// Call another service after call to this one has resolved successfully.
    ///
    /// This function can be used to chain two services together and ensure that
    /// the second service isn't called until call to the fist service have
    /// finished. Result of the call to the first service is used as an
    /// input parameter for the second service's call.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it.
    fn and_then<Next, F>(self, service: F) -> AndThen<Self, Next>
    where
        Self: IntoService<Self, Req> + Sized,
        F: IntoService<Next, Self::Response>,
        Next: Service<Self::Response, Error = Self::Error>,
    {
        AndThen::new(self.into_service(), service.into_service())
    }

    /// Chain on a computation for when a call to the service finished,
    /// passing the result of the call to the next service `U`.
    ///
    /// Note that this function consumes the receiving pipeline and returns a
    /// wrapped version of it.
    fn then<Next, F>(self, service: F) -> Then<Self, Next>
    where
        Self: IntoService<Self, Req> + Sized,
        F: IntoService<Next, Result<Self::Response, Self::Error>>,
        Next: Service<Result<Self::Response, Self::Error>, Error = Self::Error>,
    {
        Then::new(self.into_service(), service.into_service())
    }

    /// Map this service's output to a different type, returning a new service
    /// of the resulting type.
    ///
    /// This function is similar to the `Option::map` or `Iterator::map` where
    /// it will change the type of the underlying service.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it, similar to the existing `map` methods in the
    /// standard library.
    fn map<F, Res>(self, f: F) -> Map<Self, F, Req, Res>
    where
        Self: IntoService<Self, Req> + Sized,
        F: Fn(Self::Response) -> Res,
    {
        Map::new(self.into_service(), f)
    }

    /// Map this service's error to a different error, returning a new service.
    ///
    /// This function is similar to the `Result::map_err` where it will change
    /// the error type of the underlying service. This is useful for example to
    /// ensure that services have the same error type.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it.
    fn map_err<F, Err>(self, f: F) -> MapErr<Self, F, Err>
    where
        Self: IntoService<Self, Req> + Sized,
        F: Fn(Self::Error) -> Err,
    {
        MapErr::new(self.into_service(), f)
    }

    /// Use function as middleware for current service.
    ///
    /// Short version of `apply_fn(chain(...), fn)`
    fn apply_fn<F, R, In, Out, Err>(
        self,
        f: F,
    ) -> Apply<Self, Req, F, R, In, Out, Err>
    where
        Self: IntoService<Self, Req> + Sized,
        F: Fn(In, Pipeline<Self>) -> R,
        R: Future<Output = Result<Out, Err>>,
        Err: From<Self::Error>,
    {
        Apply::new(self.into_service(), f)
    }
    
    /// Create service pipeline
    fn into_pipeline(self) -> Pipeline<Self>
    where 
    Self: IntoService<Self, Req> + Sized {
        Pipeline::new(self.into_service())
    }
}

/// Factory for creating `Service`s.
///
/// This is useful for cases where new `Service`s must be produced. One case is a TCP server
/// listener: a listener accepts new connections, constructs a new `Service` for each using
/// the `ServiceFactory` trait, and uses the new `Service` to process inbound requests on that
/// new connection.
///
/// `Config` is a service factory configuration type.
///
/// Simple factories may be able to use [`fn_factory`] or [`fn_factory_with_config`] to
/// reduce boilerplate.
pub trait ServiceFactory<Req, Cfg = ()>{
    /// Responses given by the created services.
    type Response;

    /// Errors produced by the created services.
    type Error;

    /// The kind of `Service` created by this factory.
    type Service: Service<Req, Response = Self::Response, Error = Self::Error>;

    /// Errors potentially raised while building a service.
    type InitError;

    /// Create and return a new service value asynchronously.
    async fn create(&self, cfg: Cfg) -> Result<Self::Service, Self::InitError>;
    
    /// Create and return a new service value asynchronously and wrap into a container
    async fn pipeline(&self, cfg: Cfg) -> Result<Pipeline<Self::Service>, Self::InitError>
    where
        Self: Sized,
    {
        Ok(Pipeline::new(self.create(cfg).await?))
    }
        
    // fn chain_factory(self) -> ServiceChainFactory<Self, Req, Cfg> {
    //     chain_factory(self)
    // }

    /// Call another service after call to this one has resolved successfully.
    fn and_then<F, U>(
        self,
        factory: F,
    ) -> AndThenFactory<Self, U>
    where
        Self: IntoServiceFactory<Self, Req, Cfg> + Sized,
        F: IntoServiceFactory<U, Self::Response, Cfg>,
        U: ServiceFactory<Self::Response, Cfg, Error = Self::Error, InitError = Self::InitError>,
    {
        AndThenFactory::new(self.into_factory(), factory.into_factory())
    }

    /// Apply middleware to current service factory.
    ///
    /// Short version of `apply(middleware, chain_factory(...))`
    fn apply<U>(self, tr: U) -> ApplyMiddleware<U, Self, Cfg>
    where
        Self: IntoServiceFactory<Self, Req, Cfg> + Sized,
        U: Middleware<Self::Service>,
    {
        ApplyMiddleware::new(tr, self.into_factory())
    }

    /// Apply function middleware to current service factory.
    ///
    /// Short version of `apply_fn_factory(chain_factory(...), fn)`
    fn apply_fn<F, R, In, Out, Err>(
        self,
        f: F,
    ) -> ApplyFactory<Self, Req, Cfg, F, R, In, Out, Err>
    where
        Self: IntoServiceFactory<Self, Req, Cfg> + Sized,
        F: Fn(In, Pipeline<Self::Service>) -> R + Clone,
        R: Future<Output = Result<Out, Err>>,
        Err: From<Self::Error>,
    {
        ApplyFactory::new(self.into_factory(), f)
    }

    /// Create chain factory to chain on a computation for when a call to the
    /// service finished, passing the result of the call to the next
    /// service `U`.
    ///
    /// Note that this function consumes the receiving factory and returns a
    /// wrapped version of it.
    fn then<F, U>(self, factory: F) -> ThenFactory<Self, U>
    where
        Self: IntoServiceFactory<Self, Req, Cfg> + Sized,
        Cfg: Clone,
        F: IntoServiceFactory<U, Result<Self::Response, Self::Error>, Cfg>,
        U: ServiceFactory<
            Result<Self::Response, Self::Error>,
            Cfg,
            Error = Self::Error,
            InitError = Self::InitError,
        >,
    {
        ThenFactory::new(self.into_factory(), factory.into_factory())
    }

    /// Map this service's output to a different type, returning a new service
    /// of the resulting type.
    fn map<F, Res>(
        self,
        f: F,
    ) -> MapFactory<Self, F, Req, Res, Cfg>
    where
        Self: IntoServiceFactory<Self, Req, Cfg> + Sized,
        F: Fn(Self::Response) -> Res + Clone,
    {
        MapFactory::new(self.into_factory(), f)
    }

    /// Map this service's error to a different error.
    fn map_err<F, E>(
        self,
        f: F,
    ) -> MapErrFactory<Self, Req, Cfg, F, E>
    where
        Self: IntoServiceFactory<Self, Req, Cfg> + Sized,
        F: Fn(Self::Error) -> E + Clone,
    {
        MapErrFactory::new(self.into_factory(), f)
    }

    /// Map this factory's init error to a different error, returning a new factory.
    fn map_init_err<F, E>(
        self,
        f: F,
    ) -> MapInitErr<Self, Req, Cfg, F, E>
    where
        Self: IntoServiceFactory<Self, Req, Cfg> + Sized,
        F: Fn(Self::InitError) -> E + Clone,
    {
        MapInitErr::new(self.into_factory(), f)
    }
}

impl<S, Req> Service<Req> for &S
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = S::Error;

    #[inline]
    async fn ready(&self, ctx: ServiceCtx<'_, Self>) -> Result<(), S::Error> {
        ctx.ready(&**self).await
    }

    #[inline]
    fn poll(&self, cx: &mut Context<'_>) -> Result<(), S::Error> {
        (**self).poll(cx)
    }

    #[inline]
    async fn shutdown(&self) {
        (**self).shutdown().await
    }

    #[inline]
    async fn call(
        &self,
        request: Req,
        ctx: ServiceCtx<'_, Self>,
    ) -> Result<Self::Response, Self::Error> {
        ctx.call_nowait(&**self, request).await
    }
}

impl<S, Req> Service<Req> for Box<S>
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = S::Error;

    #[inline]
    async fn ready(&self, ctx: ServiceCtx<'_, Self>) -> Result<(), S::Error> {
        ctx.ready(&**self).await
    }

    #[inline]
    async fn shutdown(&self) {
        (**self).shutdown().await
    }

    #[inline]
    async fn call(
        &self,
        request: Req,
        ctx: ServiceCtx<'_, Self>,
    ) -> Result<Self::Response, Self::Error> {
        ctx.call_nowait(&**self, request).await
    }

    #[inline]
    fn poll(&self, cx: &mut Context<'_>) -> Result<(), S::Error> {
        (**self).poll(cx)
    }
}

impl<S, Req, Cfg> ServiceFactory<Req, Cfg> for Rc<S>
where
    S: ServiceFactory<Req, Cfg>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Service = S::Service;
    type InitError = S::InitError;

    async fn create(&self, cfg: Cfg) -> Result<Self::Service, Self::InitError> {
        self.as_ref().create(cfg).await
    }
    
    async fn pipeline(&self, cfg: Cfg) -> Result<Pipeline<Self::Service>, Self::InitError>
    where
        Self: Sized,
    {
        Ok(Pipeline::new(self.create(cfg).await?))
    }
}

/// Trait for types that can be converted to a `Service`
pub trait IntoService<Svc, Req>
where
    Svc: Service<Req>,
{
    /// Convert to a `Service`
    fn into_service(self) -> Svc;
}

/// Trait for types that can be converted to a `ServiceFactory`
pub trait IntoServiceFactory<T, Req, Cfg = ()>
where
    T: ServiceFactory<Req, Cfg>,
{
    /// Convert `Self` to a `ServiceFactory`
    fn into_factory(self) -> T;
}

impl<Svc, Req> IntoService<Svc, Req> for Svc
where
    Svc: Service<Req>,
{
    #[inline]
    fn into_service(self) -> Svc {
        self
    }
}

impl<T, Req, Cfg> IntoServiceFactory<T, Req, Cfg> for T
where
    T: ServiceFactory<Req, Cfg>,
{
    #[inline]
    fn into_factory(self) -> T {
        self
    }
}

pub mod dev {
    pub use crate::and_then::{AndThen, AndThenFactory};
    pub use crate::apply::{Apply, ApplyFactory};
    pub use crate::fn_service::{
        FnService, FnServiceConfig, FnServiceFactory, FnServiceNoConfig,
    };
    pub use crate::fn_shutdown::FnShutdown;
    pub use crate::map::{Map, MapFactory};
    pub use crate::map_config::{MapConfig, UnitConfig};
    pub use crate::map_err::{MapErr, MapErrFactory};
    pub use crate::map_init_err::MapInitErr;
    pub use crate::middleware::ApplyMiddleware;
    pub use crate::then::{Then, ThenFactory};
}
