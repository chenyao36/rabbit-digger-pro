use std::net::SocketAddr;

use self::source::UdpSource;
use super::wrapper::{Cipher, CryptoStream};
use rd_interface::{async_trait, prelude::*, Address, Arc, IServer, Net, Result, TcpStream};
use rd_std::util::{connect_tcp, forward_udp};
use shadowsocks::{config::ServerType, context::Context, ServerConfig};
use socks5_protocol::Address as S5Addr;
use tokio::select;

mod source;

#[rd_config]
#[derive(Debug, Clone)]
pub struct SSServerConfig {
    pub(crate) bind: Address,
    pub(crate) password: String,
    #[serde(default)]
    pub(crate) udp: bool,

    pub(crate) cipher: Cipher,
}

pub struct SSServer {
    bind: Address,
    context: Arc<Context>,
    cfg: Arc<ServerConfig>,
    listen: Net,
    net: Net,
}

#[async_trait]
impl IServer for SSServer {
    async fn start(&self) -> Result<()> {
        select! {
            r = self.serve_tcp() => r,
            r = self.serve_udp() => r,
        }
    }
}

impl SSServer {
    pub fn new(listen: Net, net: Net, cfg: SSServerConfig) -> SSServer {
        let context = Arc::new(Context::new(ServerType::Local));
        let svr_cfg =
            ServerConfig::new(("example.com", 0), cfg.password.clone(), cfg.cipher.into());

        SSServer {
            bind: cfg.bind,
            context,
            cfg: Arc::new(svr_cfg),
            listen,
            net,
        }
    }
    async fn serve_udp(&self) -> Result<()> {
        let udp_listener = self
            .listen
            .udp_bind(&mut rd_interface::Context::new(), &self.bind)
            .await?;

        forward_udp(
            UdpSource::new(
                self.cfg.method(),
                self.cfg.key().to_vec().into_boxed_slice(),
                udp_listener,
            ),
            self.net.clone(),
        )
        .await?;

        Ok(())
    }
    async fn serve_tcp(&self) -> Result<()> {
        let listener = self
            .listen
            .tcp_bind(&mut rd_interface::Context::new(), &self.bind)
            .await?;
        loop {
            let (socket, addr) = listener.accept().await?;
            let cfg = self.cfg.clone();
            let context = self.context.clone();
            let net = self.net.clone();
            let _ = tokio::spawn(async move {
                if let Err(e) = Self::serve_connection(cfg, context, socket, net, addr).await {
                    tracing::error!("Error when serve_connection: {:?}", e);
                }
            });
        }
    }
    async fn serve_connection(
        cfg: Arc<ServerConfig>,
        context: Arc<Context>,
        socket: TcpStream,
        net: Net,
        addr: SocketAddr,
    ) -> Result<()> {
        let mut socket = CryptoStream::from_stream(context, socket, cfg.method(), cfg.key());
        let target = S5Addr::read(&mut socket).await.map_err(|e| e.to_io_err())?;

        let ctx = &mut rd_interface::Context::from_socketaddr(addr);
        let target = net
            .tcp_connect(
                ctx,
                &match target {
                    S5Addr::Domain(d, p) => Address::Domain(d, p),
                    S5Addr::SocketAddr(s) => Address::SocketAddr(s),
                },
            )
            .await?;
        connect_tcp(ctx, socket, target).await?;
        Ok(())
    }
}
