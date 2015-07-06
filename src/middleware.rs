use request::Request;
use response::Response;
use nickel_error::NickelError;
use hyper::net;

pub use self::Action::{Continue, Halt};

pub type MiddlewareResult<'a, D> = Result<Action<Response<'a, D, net::Fresh>,
                                                 Response<'a, D, net::Streaming>>,
                                          NickelError<'a, D>>;

pub enum Action<T=(), U=()> {
    Continue(T),
    Halt(U)
}

// the usage of + Send is weird here because what we really want is + Static
// but that's not possible as of today. We have to use + Send for now.
pub trait Middleware<D>: Send + 'static + Sync {
    fn invoke<'a, 'b>(&'a self, _req: &mut Request<'b, 'a, 'b, D>, res: Response<'a, D, net::Fresh>) -> MiddlewareResult<'a, D> {
        Ok(Continue(res))
    }
}

impl<T, D> Middleware<D> for T where T: for<'r, 'b, 'a> Fn(&'r mut Request<'b, 'a, 'b, D>, Response<'a, D>) -> MiddlewareResult<'a, D> + Send + Sync + 'static {
    fn invoke<'a, 'b>(&'a self, req: &mut Request<'b, 'a, 'b, D>, res: Response<'a, D>) -> MiddlewareResult<'a, D> {
        (*self)(req, res)
    }
}

pub trait ErrorHandler<D>: Send + 'static + Sync {
    fn handle_error(&self, &mut NickelError<D>, &mut Request<D>) -> Action;
}

impl<D> ErrorHandler<D> for fn(&mut NickelError<D>, &mut Request<D>) -> Action {
    fn handle_error(&self, err: &mut NickelError<D>, req: &mut Request<D>) -> Action {
        (*self)(err, req)
    }
}

pub struct MiddlewareStack<D> {
    handlers: Vec<Box<Middleware<D> + Send + Sync>>,
    error_handlers: Vec<Box<ErrorHandler<D> + Send + Sync>>
}

impl<D> MiddlewareStack<D> {
    pub fn add_middleware<T: Middleware<D>> (&mut self, handler: T) {
        self.handlers.push(Box::new(handler));
    }

    pub fn add_error_handler<T: ErrorHandler<D>> (&mut self, handler: T) {
        self.error_handlers.push(Box::new(handler));
    }

    pub fn invoke<'a, 'b>(&'a self, mut req: Request<'a, 'a, 'b, D>, mut res: Response<'a, D>) {
        for handler in self.handlers.iter() {
            match handler.invoke(&mut req, res) {
                Ok(Halt(res)) => {
                    debug!("Halted {:?} {:?} {:?} {:?}",
                           req.origin.method,
                           req.origin.remote_addr,
                           req.origin.uri,
                           res.status());
                    let _ = res.end();
                    return
                }
                Ok(Continue(fresh)) => res = fresh,
                Err(mut err) => {
                    warn!("{:?} {:?} {:?} {:?} {:?}",
                          req.origin.method,
                          req.origin.remote_addr,
                          req.origin.uri,
                          err.message,
                          err.stream.as_ref().map(|s| s.status()));

                    for error_handler in self.error_handlers.iter().rev() {
                        if let Halt(()) = error_handler.handle_error(&mut err, &mut req) {
                            err.end();
                            return
                        }
                    }

                    warn!("Unhandled error: {:?} {:?} {:?} {:?} {:?}",
                          req.origin.method,
                          req.origin.remote_addr,
                          req.origin.uri,
                          err.message,
                          err.stream.map(|s| s.status()));
                    return
                }
            }
        }
    }

    pub fn new () -> MiddlewareStack<D> {
        MiddlewareStack{
            handlers: Vec::new(),
            error_handlers: Vec::new()
        }
    }
}
