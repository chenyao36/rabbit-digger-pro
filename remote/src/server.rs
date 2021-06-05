use std::sync::atomic::{AtomicU64, Ordering};

use crate::protocol::{Channel, CommandRequest, CommandResponse, Protocol};
use dashmap::DashMap;
use rd_interface::{
    async_trait, util::connect_tcp, Arc, Context, Error, IServer, Net, Result, TcpStream,
};

#[derive(Clone)]
struct Map(Arc<(DashMap<u64, TcpStream>, AtomicU64)>);

impl Map {
    fn new() -> Map {
        Map(Arc::new((DashMap::new(), AtomicU64::new(0))))
    }
    fn insert(&self, tcp: TcpStream) -> u64 {
        let id = self.0 .1.fetch_add(10, Ordering::SeqCst);
        self.0 .0.insert(id, tcp);
        id
    }
    fn get(&self, id: u64) -> Option<TcpStream> {
        self.0 .0.remove(&id).map(|i| i.1)
    }
}

pub struct RemoteServer {
    net: Net,
    protocol: Arc<dyn Protocol>,
}

#[async_trait]
impl IServer for RemoteServer {
    async fn start(&self) -> Result<()> {
        let map = Map::new();

        loop {
            let channel = self.protocol.channel().await?;
            tokio::spawn(process_channel(channel, self.net.clone(), map.clone()));
        }
    }
}

async fn process_channel(mut channel: Channel, net: Net, map: Map) -> Result<()> {
    let req: CommandRequest = channel.recv().await?;

    match req {
        CommandRequest::TcpConnect { address } => {
            let target = net.tcp_connect(&mut Context::new(), address).await?;
            connect_tcp(target, channel.into_inner()).await?;
        }
        CommandRequest::TcpBind { address } => {
            let listener = net.tcp_bind(&mut Context::new(), address).await?;
            channel
                .send(CommandResponse::BindAddr {
                    addr: listener.local_addr().await?,
                })
                .await?;

            loop {
                let (tcp, addr) = listener.accept().await?;
                let id = map.insert(tcp);
                channel.send(CommandResponse::Accept { id, addr }).await?;
            }
        }
        CommandRequest::TcpAccept { id } => {
            let target = map.get(id).ok_or(Error::Other("ID is not found".into()))?;
            connect_tcp(target, channel.into_inner()).await?;
        }
    }

    Ok(())
}

impl RemoteServer {
    pub fn new(protocol: Arc<dyn Protocol>, net: Net) -> RemoteServer {
        RemoteServer { net, protocol }
    }
}
