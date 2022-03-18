use std::collections::BTreeMap;

use crate::{config::Import, storage::Storage};
use anyhow::{anyhow, Result};
use once_cell::sync::OnceCell;
use rabbit_digger::config::Config;
use rd_interface::{
    async_trait,
    registry::{Builder, Resolver},
};

mod clash;
mod merge;
#[cfg(feature = "rhai")]
mod rhai;

type Registry = BTreeMap<&'static str, Resolver<BoxImporter>>;

pub fn get_importer_registry() -> &'static Registry {
    static REGISTRY: OnceCell<Registry> = OnceCell::new();
    REGISTRY.get_or_init(|| {
        let mut registry = BTreeMap::new();

        fn add_importer<N: Builder<BoxImporter>>(r: &mut Registry) {
            r.insert(N::NAME, Resolver::new::<N>());
        }
        add_importer::<clash::Clash>(&mut registry);
        add_importer::<merge::Merge>(&mut registry);
        #[cfg(feature = "rhai")]
        add_importer::<rhai::Rhai>(&mut registry);

        registry
    })
}

pub fn get_importer(import: &Import) -> Result<Box<dyn Importer>> {
    let registry = get_importer_registry();
    let resolver = registry
        .get(&import.format.as_ref())
        .ok_or_else(|| anyhow!("Importer not found: {}", import.format))?;

    Ok(resolver.build(&|_| None, import.opt.clone())?)
}

#[async_trait]
pub trait Importer: Send + Sync {
    async fn process(
        &mut self,
        config: &mut Config,
        content: &str,
        cache: &dyn Storage,
    ) -> Result<()>;
}
pub type BoxImporter = Box<dyn Importer>;
