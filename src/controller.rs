mod event;
mod wrapper;

use crate::{
    config,
    rabbit_digger::{RabbitDigger, RabbitDiggerBuilder},
    Registry,
};

use self::event::{BatchEvent, Event, EventType};
use anyhow::{anyhow, Result};
use futures::{
    future::{ready, try_select, Either},
    pin_mut, stream, FutureExt, Stream, StreamExt, TryStreamExt,
};
use rd_interface::{
    async_trait, schemars::schema::RootSchema, Address, Context, INet, IntoDyn, Net, TcpListener,
    TcpStream, UdpSocket,
};
use serde_derive::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{sync::broadcast, time::timeout};
use tokio::{
    sync::mpsc,
    sync::{RwLock, RwLockReadGuard},
    task::spawn,
    time::sleep,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct RegistrySchema {
    net: HashMap<String, RootSchema>,
    server: HashMap<String, RootSchema>,
}

pub struct Inner {
    sender: broadcast::Sender<BatchEvent>,
    builder: RabbitDiggerBuilder,
    state: State,
}

#[derive(Debug)]
pub struct TaskInfo {
    pub name: String,
}

#[derive(Debug)]
pub struct Running {
    config: config::Config,
    registry: RegistrySchema,
}

#[derive(Debug)]
pub enum State {
    Idle,
    Running(Running),
}

impl State {
    fn running(&self) -> Option<&Running> {
        match self {
            State::Running(r) => Some(r),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Controller {
    inner: Arc<RwLock<Inner>>,
    event_sender: mpsc::UnboundedSender<Event>,
}

pub struct ControllerNet {
    net: Net,
    sender: mpsc::UnboundedSender<Event>,
}

#[async_trait]
impl INet for ControllerNet {
    async fn tcp_connect(
        &self,
        ctx: &mut Context,
        addr: Address,
    ) -> rd_interface::Result<TcpStream> {
        let tcp = self.net.tcp_connect(ctx, addr.clone()).await?;
        let tcp = wrapper::TcpStream::new(tcp, self.sender.clone());
        tcp.send(EventType::NewTcp(addr));
        Ok(tcp.into_dyn())
    }

    // TODO: wrap TcpListener
    async fn tcp_bind(
        &self,
        ctx: &mut Context,
        addr: Address,
    ) -> rd_interface::Result<TcpListener> {
        self.net.tcp_bind(ctx, addr).await
    }

    // TODO: wrap UdpSocket
    async fn udp_bind(&self, ctx: &mut Context, addr: Address) -> rd_interface::Result<UdpSocket> {
        self.net.udp_bind(ctx, addr).await
    }
}

async fn process(mut rx: mpsc::UnboundedReceiver<Event>, sender: broadcast::Sender<BatchEvent>) {
    loop {
        let e = match rx.recv().now_or_never() {
            Some(Some(e)) => e,
            Some(None) => break,
            None => {
                sleep(Duration::from_millis(100)).await;
                continue;
            }
        };

        let mut events = BatchEvent::with_capacity(16);
        events.push(Arc::new(e));
        while let Some(Some(e)) = rx.recv().now_or_never() {
            events.push(Arc::new(e));
        }

        // Failed only when no receiver
        sender.send(events).ok();
    }
}

impl Controller {
    pub fn new() -> Controller {
        let (sender, _) = broadcast::channel(16);
        let inner = Arc::new(RwLock::new(Inner {
            sender: sender.clone(),
            state: State::Idle,
            builder: RabbitDiggerBuilder::new(),
        }));
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        spawn(process(event_receiver, sender));
        Controller {
            inner,
            event_sender,
        }
    }

    pub async fn run(&self, config: config::Config) -> Result<()> {
        let config_stream = stream::once(ready(Ok(config)));
        self.run_stream(config_stream).await
    }

    pub async fn run_stream<S>(&self, config_stream: S) -> Result<()>
    where
        S: Stream<Item = Result<config::Config>>,
    {
        futures::pin_mut!(config_stream);

        let mut config = match timeout(Duration::from_secs(1), config_stream.try_next()).await {
            Ok(Ok(Some(cfg))) => cfg,
            Ok(Err(e)) => return Err(e.context(format!("Failed to get first config."))),
            Err(_) | Ok(Ok(None)) => {
                return Err(anyhow!("The config_stream is empty, can not start."))
            }
        };
        let mut config_stream = config_stream.chain(stream::pending());

        loop {
            log::info!("rabbit digger is starting...");

            let RabbitDigger {
                config: rd_config,
                registry,
                servers,
                ..
            } = self.inner.read().await.builder.build(self, config)?;

            self.inner.write().await.state = State::Running(Running {
                config: rd_config,
                registry: get_registry_schema(&registry)?,
            });

            let run_fut = RabbitDigger::run(servers);
            pin_mut!(run_fut);
            let new_config = match try_select(run_fut, config_stream.try_next()).await {
                Ok(Either::Left((_, cfg_fut))) => {
                    log::info!("Exited normally, waiting for next config...");
                    cfg_fut.await
                }
                Ok(Either::Right((cfg, _))) => Ok(cfg),
                Err(Either::Left((e, cfg_fut))) => {
                    log::error!(
                        "Rabbit digger went to error: {:?}, waiting for next config...",
                        e
                    );
                    cfg_fut.await
                }
                Err(Either::Right((e, _))) => Err(e),
            };

            self.inner.write().await.state = State::Idle;

            config = match new_config? {
                Some(v) => v,
                None => break,
            }
        }

        Ok(())
    }

    pub fn get_net(&self, net: Net) -> Net {
        ControllerNet {
            net,
            sender: self.event_sender.clone(),
        }
        .into_dyn()
    }

    pub async fn set_plugin_loader(
        &self,
        plugin_loader: impl Fn(&config::Config, &mut Registry) -> Result<()> + Send + Sync + 'static,
    ) {
        self.inner.write().await.builder.plugin_loader = Arc::new(plugin_loader);
    }
    pub async fn lock<'a>(&'a self) -> RwLockReadGuard<'a, Inner> {
        self.inner.read().await
    }
    pub async fn get_subscriber(&self) -> broadcast::Receiver<BatchEvent> {
        self.inner.read().await.sender.subscribe()
    }
}

impl Inner {
    pub fn config(&self) -> Option<&config::Config> {
        self.state.running().map(|i| &i.config)
    }
    pub fn registry(&self) -> Option<&RegistrySchema> {
        self.state.running().map(|i| &i.registry)
    }
    pub fn state(&self) -> &'static str {
        match self.state {
            State::Idle => "Idle",
            State::Running(_) => "Running",
        }
    }
}

fn get_registry_schema(registry: &Registry) -> Result<RegistrySchema> {
    let mut r = RegistrySchema {
        net: HashMap::new(),
        server: HashMap::new(),
    };

    for (key, value) in &registry.net {
        r.net.insert(key.clone(), value.resolver.schema().clone());
    }
    for (key, value) in &registry.server {
        r.server
            .insert(key.clone(), value.resolver.schema().clone());
    }

    Ok(r)
}
