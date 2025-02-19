#![allow(clippy::type_complexity)]
use std::future::Future;

use crate::and_then::{AndThen, AndThenFactory};
use crate::apply::{Apply, ApplyFactory};
use crate::map::{Map, MapFactory};
use crate::map_err::{MapErr, MapErrFactory};
use crate::map_init_err::MapInitErr;
use crate::middleware::{ApplyMiddleware, Middleware};
use crate::then::{Then, ThenFactory};
use crate::{IntoService, IntoServiceFactory, Pipeline, Service, ServiceFactory};

// /// Constructs new chain with one service.
// pub fn chain<Svc, Req, F>(service: F) -> ServiceChain<Svc, Req>
// where
//     Svc: Service<Req>,
//     F: IntoService<Svc, Req>,
// {
//     ServiceChain {
//         service: service.into_service(),
//         _t: PhantomData,
//     }
// }

// /// Constructs new chain factory with one service factory.
// pub fn chain_factory<T, R, C, F>(factory: F) -> ServiceChainFactory<T, R, C>
// where
//     T: ServiceFactory<R, C>,
//     F: IntoServiceFactory<T, R, C>,
// {
//     ServiceChainFactory {
//         factory: factory.into_factory(),
//         _t: PhantomData,
//     }
// }

// /// Chain builder - chain allows to compose multiple service into one service.
// pub struct ServiceChain<Svc, Req> {
//     service: Svc,
//     _t: PhantomData<Req>,
// }

// impl<Svc, Req> Clone for ServiceChain<Svc, Req>
// where
//     Svc: Clone,
// {
//     fn clone(&self) -> Self {
//         ServiceChain {
//             service: self.service.clone(),
//             _t: PhantomData,
//         }
//     }
// }

// impl<Svc, Req> fmt::Debug for ServiceChain<Svc, Req>
// where
//     Svc: fmt::Debug,
// {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         f.debug_struct("ServiceChain")
//             .field("service", &self.service)
//             .finish()
//     }
// }

// impl<Svc: Service<Req>, Req> IntoService<Svc, Req> for ServiceChain<Svc, Req> {
//     fn into_service(self) -> Svc {
//         self.service
//     }
// }

// /// Service factory builder
// pub struct ServiceChainFactory<T, Req, C = ()> {
//     factory: T,
//     _t: PhantomData<(Req, C)>,
// }

// impl<T, R, C> Clone for ServiceChainFactory<T, R, C>
// where
//     T: Clone,
// {
//     fn clone(&self) -> Self {
//         ServiceChainFactory {
//             factory: self.factory.clone(),
//             _t: PhantomData,
//         }
//     }
// }

// impl<T, R, C> fmt::Debug for ServiceChainFactory<T, R, C>
// where
//     T: fmt::Debug,
// {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         f.debug_struct("ServiceChainFactory")
//             .field("factory", &self.factory)
//             .finish()
//     }
// }

// impl<T: ServiceFactory<R, C>, R, C> IntoServiceFactory<T, R, C> for ServiceChainFactory<T, R, C> {
//     fn into_factory(self) -> T {
//         self.factory
//     }
// }

// impl<T: ServiceFactory<R, C>, R, C> AsServiceFactory<T, R, C> for ServiceChainFactory<T, R, C> {
//     fn as_factory(&self) -> &T {
//         &self.factory
//     }
// }

/// Methods for chaining services together
pub trait ChainService<Svc, Req>
where
    Svc: Service<Req>,
{
    /// Call another service after call to this one has resolved successfully.
    ///
    /// This function can be used to chain two services together and ensure that
    /// the second service isn't called until call to the fist service have
    /// finished. Result of the call to the first service is used as an
    /// input parameter for the second service's call.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it.
    fn and_then<Next, F>(self, service: F) -> AndThen<Svc, Next>
    where
        Self: Sized,
        F: IntoService<Next, Svc::Response>,
        Next: Service<Svc::Response, Error = Svc::Error>;

    /// Chain on a computation for when a call to the service finished,
    /// passing the result of the call to the next service `U`.
    ///
    /// Note that this function consumes the receiving pipeline and returns a
    /// wrapped version of it.
    fn then<Next, F>(self, service: F) -> Then<Svc, Next>
    where
        Self: Sized,
        F: IntoService<Next, Result<Svc::Response, Svc::Error>>,
        Next: Service<Result<Svc::Response, Svc::Error>, Error = Svc::Error>;

    /// Map this service's output to a different type, returning a new service
    /// of the resulting type.
    ///
    /// This function is similar to the `Option::map` or `Iterator::map` where
    /// it will change the type of the underlying service.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it, similar to the existing `map` methods in the
    /// standard library.
    fn map<F, Res>(self, f: F) -> Map<Svc, F, Req, Res>
    where
        Self: Sized,
        F: Fn(Svc::Response) -> Res;

    /// Map this service's error to a different error, returning a new service.
    ///
    /// This function is similar to the `Result::map_err` where it will change
    /// the error type of the underlying service. This is useful for example to
    /// ensure that services have the same error type.
    ///
    /// Note that this function consumes the receiving service and returns a
    /// wrapped version of it.
    fn map_err<F, Err>(self, f: F) -> MapErr<Svc, F, Err>
    where
        Self: Sized,
        F: Fn(Svc::Error) -> Err;

    /// Use function as middleware for current service.
    ///
    /// Short version of `apply_fn(chain(...), fn)`
    fn apply_fn<F, R, In, Out, Err>(
        self,
        f: F,
    ) -> Apply<Svc, Req, F, R, In, Out, Err>
    where
        Self: Sized,
        F: Fn(In, Pipeline<Svc>) -> R,
        R: Future<Output = Result<Out, Err>>,
        Svc: Service<Req>,
        Err: From<Svc::Error>;

    /// Create service pipeline
    fn into_pipeline(self) -> Pipeline<Svc>
    where
        Self: Sized;
}

/// Methods for chaining services together
impl<T, Svc, Req> ChainService<Svc, Req> for T
where
    Svc: Service<Req>,
    T: IntoService<Svc, Req>
{
    fn and_then<Next, F>(self, service: F) -> AndThen<Svc, Next>
    where
        Self: Sized,
        F: IntoService<Next, Svc::Response>,
        Next: Service<Svc::Response, Error = Svc::Error>,
    {
        AndThen::new(self.into_service(), service.into_service())
    }

    fn then<Next, F>(self, service: F) -> Then<Svc, Next>
    where
        Self: Sized,
        F: IntoService<Next, Result<Svc::Response, Svc::Error>>,
        Next: Service<Result<Svc::Response, Svc::Error>, Error = Svc::Error>,
    {
        Then::new(self.into_service(), service.into_service())
    }

    fn map<F, Res>(self, f: F) -> Map<Svc, F, Req, Res>
    where
        Self: Sized,
        F: Fn(Svc::Response) -> Res,
    {
        Map::new(self.into_service(), f)
    }

    fn map_err<F, Err>(self, f: F) -> MapErr<Svc, F, Err>
    where
        Self: Sized,
        F: Fn(Svc::Error) -> Err,
    {
        MapErr::new(self.into_service(), f)
    }

    fn apply_fn<F, R, In, Out, Err>(
        self,
        f: F,
    ) -> Apply<Svc, Req, F, R, In, Out, Err>
    where
        Self: Sized,
        F: Fn(In, Pipeline<Svc>) -> R,
        R: Future<Output = Result<Out, Err>>,
        Svc: Service<Req>,
        Err: From<Svc::Error>,
    {
        Apply::new(self.into_service(), f)
    }
    
    fn into_pipeline(self) -> Pipeline<Svc> {
        Pipeline::new(self.into_service())
    }
}

pub trait ChainServiceFactory<T, Req, C>
where 
    T: ServiceFactory<Req, C>, 
{
    /// Call another service after call to this one has resolved successfully.
    fn and_then<F, U>(
        self,
        factory: F,
    ) -> AndThenFactory<T, U>
    where
        Self: Sized,
        F: IntoServiceFactory<U, T::Response, C>,
        U: ServiceFactory<T::Response, C, Error = T::Error, InitError = T::InitError>;

    /// Apply middleware to current service factory.
    ///
    /// Short version of `apply(middleware, chain_factory(...))`
    fn apply<U>(self, tr: U) -> ApplyMiddleware<U, T, C>
    where
        U: Middleware<T::Service>;

    /// Apply function middleware to current service factory.
    ///
    /// Short version of `apply_fn_factory(chain_factory(...), fn)`
    fn apply_fn<F, R, In, Out, Err>(
        self,
        f: F,
    ) -> ApplyFactory<T, Req, C, F, R, In, Out, Err>
    where
        F: Fn(In, Pipeline<T::Service>) -> R + Clone,
        R: Future<Output = Result<Out, Err>>,
        Err: From<T::Error>;

    /// Create chain factory to chain on a computation for when a call to the
    /// service finished, passing the result of the call to the next
    /// service `U`.
    ///
    /// Note that this function consumes the receiving factory and returns a
    /// wrapped version of it.
    fn then<F, U>(self, factory: F) -> ThenFactory<T, U>
    where
        Self: Sized,
        C: Clone,
        F: IntoServiceFactory<U, Result<T::Response, T::Error>, C>,
        U: ServiceFactory<
            Result<T::Response, T::Error>,
            C,
            Error = T::Error,
            InitError = T::InitError,
        >;

    /// Map this service's output to a different type, returning a new service
    /// of the resulting type.
    fn map<F, Res>(
        self,
        f: F,
    ) -> MapFactory<T, F, Req, Res, C>
    where
        Self: Sized,
        F: Fn(T::Response) -> Res + Clone;

    /// Map this service's error to a different error.
    fn map_err<F, E>(
        self,
        f: F,
    ) -> MapErrFactory<T, Req, C, F, E>
    where
        Self: Sized,
        F: Fn(T::Error) -> E + Clone;

    /// Map this factory's init error to a different error, returning a new factory.
    fn map_init_err<F, E>(
        self,
        f: F,
    ) -> MapInitErr<T, Req, C, F, E>
    where
        Self: Sized,
        F: Fn(T::InitError) -> E + Clone;
}

impl<T, SvcFact, Req, C> ChainServiceFactory<SvcFact, Req, C> for T
where 
    T: IntoServiceFactory<SvcFact, Req, C>,
    SvcFact: ServiceFactory<Req, C> {
    /// Call another service after call to this one has resolved successfully.
    fn and_then<F, U>(
        self,
        factory: F,
    ) -> AndThenFactory<SvcFact, U>
    where
        Self: Sized,
        F: IntoServiceFactory<U, SvcFact::Response, C>,
        U: ServiceFactory<SvcFact::Response, C, Error = SvcFact::Error, InitError = SvcFact::InitError>,
    {
        AndThenFactory::new(self.into_factory(), factory.into_factory())
    }

    /// Apply middleware to current service factory.
    ///
    /// Short version of `apply(middleware, chain_factory(...))`
    fn apply<U>(self, tr: U) -> ApplyMiddleware<U, SvcFact, C>
    where
        U: Middleware<SvcFact::Service>,
    {
        ApplyMiddleware::new(tr, self.into_factory())
    }

    /// Apply function middleware to current service factory.
    ///
    /// Short version of `apply_fn_factory(chain_factory(...), fn)`
    fn apply_fn<F, R, In, Out, Err>(
        self,
        f: F,
    ) -> ApplyFactory<SvcFact, Req, C, F, R, In, Out, Err>
    where
        F: Fn(In, Pipeline<SvcFact::Service>) -> R + Clone,
        R: Future<Output = Result<Out, Err>>,
        Err: From<SvcFact::Error>,
    {
        ApplyFactory::new(self.into_factory(), f)
    }

    /// Create chain factory to chain on a computation for when a call to the
    /// service finished, passing the result of the call to the next
    /// service `U`.
    ///
    /// Note that this function consumes the receiving factory and returns a
    /// wrapped version of it.
    fn then<F, U>(self, factory: F) -> ThenFactory<SvcFact, U>
    where
        Self: Sized,
        C: Clone,
        F: IntoServiceFactory<U, Result<SvcFact::Response, SvcFact::Error>, C>,
        U: ServiceFactory<
            Result<SvcFact::Response, SvcFact::Error>,
            C,
            Error = SvcFact::Error,
            InitError = SvcFact::InitError,
        >,
    {
        ThenFactory::new(self.into_factory(), factory.into_factory())
    }

    /// Map this service's output to a different type, returning a new service
    /// of the resulting type.
    fn map<F, Res>(
        self,
        f: F,
    ) -> MapFactory<SvcFact, F, Req, Res, C>
    where
        Self: Sized,
        F: Fn(SvcFact::Response) -> Res + Clone,
    {
        MapFactory::new(self.into_factory(), f)
    }

    /// Map this service's error to a different error.
    fn map_err<F, E>(
        self,
        f: F,
    ) -> MapErrFactory<SvcFact, Req, C, F, E>
    where
        Self: Sized,
        F: Fn(SvcFact::Error) -> E + Clone,
    {
        MapErrFactory::new(self.into_factory(), f)
    }

    /// Map this factory's init error to a different error, returning a new factory.
    fn map_init_err<F, E>(
        self,
        f: F,
    ) -> MapInitErr<SvcFact, Req, C, F, E>
    where
        Self: Sized,
        F: Fn(SvcFact::InitError) -> E + Clone,
    {
        MapInitErr::new(self.into_factory(), f)
    }
}