use std::{
    future::pending,
    io,
    net::SocketAddr,
    pin::Pin,
    task::{self, Poll},
};

use crate::util::{connect_tcp, connect_udp};
use futures::{ready, Sink, SinkExt, Stream, StreamExt};
use rd_interface::{
    async_trait, prelude::*, registry::ServerBuilder, Address, Arc, Bytes, BytesMut, Context,
    IServer, IUdpChannel, IntoDyn, Net, Result, TcpListener, TcpStream, UdpSocket,
};
use tokio::select;

/// A server that forwards all connections to target.
#[rd_config]
#[derive(Debug)]
pub struct ForwardServerConfig {
    bind: Address,
    target: Address,
    #[serde(default)]
    udp: bool,
}

pub struct ForwardServer {
    listen_net: Net,
    net: Net,
    cfg: Arc<ForwardServerConfig>,
}

impl ForwardServer {
    fn new(listen_net: Net, net: Net, cfg: ForwardServerConfig) -> ForwardServer {
        ForwardServer {
            listen_net,
            net,
            cfg: Arc::new(cfg),
        }
    }
}
#[async_trait]
impl IServer for ForwardServer {
    async fn start(&self) -> Result<()> {
        let listener = self
            .listen_net
            .tcp_bind(&mut Context::new(), &self.cfg.bind)
            .await?;

        let tcp_task = self.serve_listener(listener);
        let udp_task = self.serve_udp();

        select! {
            r = tcp_task => r?,
            r = udp_task => r?,
        }

        Ok(())
    }
}

impl ForwardServer {
    async fn serve_connection(
        cfg: Arc<ForwardServerConfig>,
        socket: TcpStream,
        net: Net,
        addr: SocketAddr,
    ) -> Result<()> {
        let ctx = &mut Context::from_socketaddr(addr);
        let target = net.tcp_connect(ctx, &cfg.target).await?;
        connect_tcp(ctx, socket, target).await?;
        Ok(())
    }
    pub async fn serve_listener(&self, listener: TcpListener) -> Result<()> {
        loop {
            let (socket, addr) = listener.accept().await?;
            let cfg = self.cfg.clone();
            let net = self.net.clone();
            let _ = tokio::spawn(async move {
                if let Err(e) = Self::serve_connection(cfg, socket, net, addr).await {
                    tracing::error!("Error when serve_connection: {:?}", e);
                }
            });
        }
    }
    async fn serve_udp(&self) -> Result<()> {
        if !self.cfg.udp {
            pending::<()>().await;
        }

        let udp_listener = ListenUdpChannel {
            udp: self
                .listen_net
                .udp_bind(&mut Context::new(), &self.cfg.bind)
                .await?,
            client: None,
            cfg: self.cfg.clone(),
        }
        .into_dyn();

        let udp = self
            .net
            .udp_bind(&mut Context::new(), &self.cfg.target.to_any_addr_port()?)
            .await?;

        connect_udp(&mut Context::new(), udp_listener, udp).await?;

        Ok(())
    }
}

impl ServerBuilder for ForwardServer {
    const NAME: &'static str = "forward";
    type Config = ForwardServerConfig;
    type Server = Self;

    fn build(listen: Net, net: Net, cfg: Self::Config) -> Result<Self> {
        Ok(ForwardServer::new(listen, net, cfg))
    }
}

struct ListenUdpChannel {
    udp: UdpSocket,
    client: Option<SocketAddr>,
    cfg: Arc<ForwardServerConfig>,
}

impl Stream for ListenUdpChannel {
    type Item = io::Result<(Bytes, Address)>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        let item = ready!(self.udp.poll_next_unpin(cx));
        Poll::Ready(item.map(|r| {
            r.map(|(bytes, addr)| {
                self.client = Some(addr);
                return (bytes.freeze(), self.cfg.target.clone());
            })
        }))
    }
}

impl Sink<(BytesMut, SocketAddr)> for ListenUdpChannel {
    type Error = io::Error;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.udp.poll_ready_unpin(cx)
    }

    fn start_send(
        mut self: Pin<&mut Self>,
        (bytes, _): (BytesMut, SocketAddr),
    ) -> Result<(), Self::Error> {
        if let Some(client) = self.client {
            self.udp.start_send_unpin((bytes.freeze(), client.into()))
        } else {
            Ok(())
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.udp.poll_flush_unpin(cx)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.udp.poll_close_unpin(cx)
    }
}

impl IUdpChannel for ListenUdpChannel {}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rd_interface::{IntoAddress, IntoDyn};
    use tokio::time::sleep;

    use super::*;
    use crate::tests::{
        assert_echo, assert_echo_udp, spawn_echo_server, spawn_echo_server_udp, TestNet,
    };

    #[tokio::test]
    async fn test_forward_server() {
        let net = TestNet::new().into_dyn();
        let cfg = ForwardServerConfig {
            bind: "127.0.0.1:1234".into_address().unwrap(),
            target: "127.0.0.1:4321".into_address().unwrap(),
            udp: true,
        };
        let server = ForwardServer::new(net.clone(), net.clone(), cfg);
        tokio::spawn(async move { server.start().await.unwrap() });
        spawn_echo_server(&net, "127.0.0.1:4321").await;
        spawn_echo_server_udp(&net, "127.0.0.1:4321").await;

        sleep(Duration::from_millis(1)).await;

        assert_echo(&net, "127.0.0.1:1234").await;
        assert_echo_udp(&net, "127.0.0.1:1234").await;
    }
}
