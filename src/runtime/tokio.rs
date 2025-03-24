use std::error::Error;

use http_body::Body;
use hyper::body::Incoming;
use hyper::service::Service;
use hyper::{Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::TcpListener;
use tracing::info;

use crate::LayaOptions;

pub fn serve<S, B>(options: LayaOptions, service: S)
where
    S: Service<Request<Incoming>, Response = Response<B>> + Send + Clone + 'static,
    S::Future: 'static,
    S::Error: Into<Box<dyn Error + Send + Sync>>,
    B: Body + Send + 'static,
    B::Data: Send + 'static,
    B::Error: Into<Box<dyn Error + Send + Sync>>,
    S::Error: Error + Send + Sync + 'static,
    S::Future: Send + 'static,
{
    let rt = tokio::runtime::Builder::new_multi_thread()
        .thread_name("laya")
        .enable_all()
        .build()
        .expect("failed to create HTTP runtime");

    let result: Result<_, std::io::Error> = rt.block_on(async move {
        info!("Starting HTTP server");
        let listener = TcpListener::bind(options.bind_address).await?;
        info!("Listening on {:?}", options.bind_address);

        loop {
            let (stream, _addr) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let service = service.clone();
            let handler = async move {
                hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                    .serve_connection(io, service)
                    .await
            };

            tokio::spawn(handler);
        }

        Ok(())
    });

    result.expect("failed to bind HTTP server")
}
