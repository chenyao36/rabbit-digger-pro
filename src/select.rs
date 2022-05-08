use rd_interface::{
    async_trait,
    prelude::*,
    registry::{Builder, NetRef},
    Error, INet, Net, Registry, Result,
};

#[rd_config]
#[derive(Debug, Clone)]
pub struct SelectNetConfig {
    selected: NetRef,
    list: Vec<NetRef>,
}

pub struct SelectNet {
    selected: Net,
}

impl SelectNet {
    pub fn new(config: SelectNetConfig) -> Result<Self> {
        if config.list.is_empty() {
            return Err(Error::Other("select list is empty".into()));
        }

        Ok(SelectNet {
            selected: (*config.selected).clone(),
        })
    }
    fn net(&self) -> Option<&Net> {
        Some(&self.selected)
    }
}

#[async_trait]
impl INet for SelectNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        self.net()?.provide_tcp_connect()
    }

    fn provide_tcp_bind(&self) -> Option<&dyn rd_interface::TcpBind> {
        self.net()?.provide_tcp_bind()
    }

    fn provide_udp_bind(&self) -> Option<&dyn rd_interface::UdpBind> {
        self.net()?.provide_udp_bind()
    }

    fn provide_lookup_host(&self) -> Option<&dyn rd_interface::LookupHost> {
        self.net()?.provide_lookup_host()
    }
}

impl Builder<Net> for SelectNet {
    const NAME: &'static str = "select";
    type Config = SelectNetConfig;
    type Item = Self;

    fn build(config: Self::Config) -> Result<Self> {
        SelectNet::new(config)
    }
}

pub fn init(registry: &mut Registry) -> Result<()> {
    registry.add_net::<SelectNet>();
    Ok(())
}
