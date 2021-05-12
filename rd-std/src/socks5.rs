mod client;
mod common;
pub mod protocol;
mod server;

pub use client::Socks5Client;
pub use server::Socks5Server;

use rd_interface::{
    registry::{NetFactory, ServerFactory},
    util::get_one_net,
    Net, Registry, Result,
};
use serde_derive::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    address: String,
    port: u16,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    bind: String,
}

impl NetFactory for Socks5Client {
    const NAME: &'static str = "socks5";
    type Config = Config;
    type Net = Self;

    fn new(net: Vec<rd_interface::Net>, config: Self::Config) -> Result<Self> {
        Ok(Socks5Client::new(
            get_one_net(net)?,
            config.address,
            config.port,
        ))
    }
}

impl ServerFactory for server::Socks5 {
    const NAME: &'static str = "socks5";
    type Config = ServerConfig;
    type Server = Self;

    fn new(listen_net: Net, net: Net, Self::Config { bind }: Self::Config) -> Result<Self> {
        Ok(server::Socks5::new(listen_net, net, bind))
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<Socks5Client>();
    registry.add_server::<server::Socks5>();
    Ok(())
}
