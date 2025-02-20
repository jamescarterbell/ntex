#![allow(clippy::type_complexity)]
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;

use crate::and_then::{AndThen, AndThenFactory};
use crate::apply::{Apply, ApplyFactory};
use crate::map::{Map, MapFactory};
use crate::map_err::{MapErr, MapErrFactory};
use crate::map_init_err::MapInitErr;
use crate::middleware::{ApplyMiddleware, Middleware};
use crate::then::{Then, ThenFactory};
use crate::{IntoService, IntoServiceFactory, Pipeline, Service, ServiceFactory};

/// Constructs new chain with one service.
pub(crate) fn chain<Svc, Req, F>(service: F) -> ServiceChain<Svc, Req>
where
    Svc: Service<Req>,
    F: IntoService<Svc, Req>,
{
    ServiceChain {
        service: service.into_service(),
        _t: PhantomData,
    }
}

/// Constructs new chain factory with one service factory.
pub(crate) fn chain_factory<T, R, C, F>(factory: F) -> ServiceChainFactory<T, R, C>
where
    T: ServiceFactory<R, C>,
    F: IntoServiceFactory<T, R, C>,
{
    ServiceChainFactory {
        factory: factory.into_factory(),
        _t: PhantomData,
    }
}

/// Chain builder - chain allows to compose multiple service into one service.
pub struct ServiceChain<Svc, Req> {
    service: Svc,
    _t: PhantomData<Req>,
}

impl<Svc, Req> Clone for ServiceChain<Svc, Req>
where
    Svc: Clone,
{
    fn clone(&self) -> Self {
        ServiceChain {
            service: self.service.clone(),
            _t: PhantomData,
        }
    }
}

impl<Svc, Req> fmt::Debug for ServiceChain<Svc, Req>
where
    Svc: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceChain")
            .field("service", &self.service)
            .finish()
    }
}

impl<Svc: Service<Req>, Req> IntoService<Svc, Req> for ServiceChain<Svc, Req> {
    fn into_service(self) -> Svc {
        self.service
    }
}

/// Service factory builder
pub struct ServiceChainFactory<T, Req, C = ()> {
    factory: T,
    _t: PhantomData<(Req, C)>,
}

impl<T, R, C> Clone for ServiceChainFactory<T, R, C>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        ServiceChainFactory {
            factory: self.factory.clone(),
            _t: PhantomData,
        }
    }
}

impl<T, R, C> fmt::Debug for ServiceChainFactory<T, R, C>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceChainFactory")
            .field("factory", &self.factory)
            .finish()
    }
}

impl<T: ServiceFactory<R, C>, R, C> IntoServiceFactory<T, R, C> for ServiceChainFactory<T, R, C> {
    fn into_factory(self) -> T {
        self.factory
    }
}
