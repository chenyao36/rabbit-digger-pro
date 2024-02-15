use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cfg_if::cfg_if;
use clap::Parser;
use futures::{
    pin_mut,
    stream::{select, TryStreamExt},
    StreamExt,
};
use rabbit_digger_pro::{config::ImportSource, schema, util::exit_stream, ApiServer, App};
use tracing_subscriber::filter::dynamic_filter_fn;

#[cfg(feature = "telemetry")]
mod tracing_helper;

#[derive(Parser)]
struct ApiServerArgs {
    /// HTTP endpoint bind address.
    #[clap(short, long, env = "RD_BIND")]
    bind: Option<String>,

    /// Access token.
    #[structopt(long, env = "RD_ACCESS_TOKEN")]
    access_token: Option<String>,

    /// Web UI. Folder path.
    #[structopt(long, env = "RD_WEB_UI")]
    web_ui: Option<String>,
}

#[derive(Parser)]
struct Args {
    /// Path to config file
    #[clap(short, long, env = "RD_CONFIG", default_value = "config.yaml")]
    config: PathBuf,

    #[clap(flatten)]
    api_server: ApiServerArgs,

    /// Write generated config to path
    #[clap(long)]
    write_config: Option<PathBuf>,

    #[clap(subcommand)]
    cmd: Option<Command>,
}

#[derive(Parser)]
enum Command {
    /// Generate schema to path, if not present, output to stdout
    GenerateSchema { path: Option<PathBuf> },
    /// Run in server mode
    Server {
        #[clap(flatten)]
        api_server: ApiServerArgs,
    },
}

impl ApiServerArgs {
    fn to_api_server(&self) -> ApiServer {
        ApiServer {
            bind: self.bind.clone(),
            access_token: self.access_token.clone(),
            web_ui: self.web_ui.clone(),
        }
    }
}

async fn write_config(path: impl AsRef<Path>, cfg: &rabbit_digger::Config) -> Result<()> {
    let content = serde_yaml::to_string(cfg)?;
    tokio::fs::write(path, content.as_bytes()).await?;
    Ok(())
}

async fn real_main(args: Args) -> Result<()> {
    let app = App::new().await?;

    app.run_api_server(args.api_server.to_api_server()).await?;

    let config_path = args.config.clone();
    let write_config_path = args.write_config;

    let config_stream = app
        .cfg_mgr
        .config_stream(ImportSource::Path(config_path))
        .await?
        .and_then(|c: rabbit_digger::Config| async {
            if let Some(path) = &write_config_path {
                write_config(path, &c).await?;
            };
            Ok(c)
        });
    let exit_stream = exit_stream().map(|i| {
        let r: Result<rabbit_digger::Config> = match i {
            Ok(_) => Err(rd_interface::Error::AbortedByUser.into()),
            Err(e) => Err(e.into()),
        };
        r
    });

    let stream = select(config_stream, exit_stream);

    pin_mut!(stream);
    app.rd
        .start_stream(stream)
        .await
        .context("Failed to run RabbitDigger")?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    use tracing_subscriber::{layer::SubscriberExt, prelude::*, EnvFilter};
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var(
            "RUST_LOG",
            "rabbit_digger=debug,rabbit_digger_pro=debug,rd_std=debug,raw=debug,ss=debug,tower_http=info",
        )
    }
    let tr = tracing_subscriber::registry();

    cfg_if! {
        if #[cfg(feature = "console")] {
            let (layer, server) = console_subscriber::ConsoleLayer::builder().with_default_env().build();
            tokio::spawn(server.serve());
            let tr = tr.with(layer);
        }
    }

    cfg_if! {
        if #[cfg(feature = "telemetry")] {
            let tracer = opentelemetry_jaeger::new_pipeline()
                .with_service_name("rabbit_digger_pro")
                .install_batch(opentelemetry::runtime::Tokio)?;
            // only for debug
            // let tracer = opentelemetry::sdk::export::trace::stdout::new_pipeline().install_simple();
            let tracer_filter =
                EnvFilter::new("rabbit_digger=trace,rabbit_digger_pro=trace,rd_std=trace");
            let opentelemetry = tracing_opentelemetry::layer().with_tracer(tracer);
            let tr = tr.with(
                opentelemetry.with_filter(dynamic_filter_fn(move |metadata, ctx| {
                    tracer_filter.enabled(metadata, ctx.clone())
                })),
            );
        }
    }

    let log_filter = EnvFilter::from_default_env();
    let log_writer_filter = EnvFilter::new(
        "rabbit_digger=debug,rabbit_digger_pro=debug,rd_std=debug,raw=debug,ss=debug",
    );
    let json_layer = tracing_subscriber::fmt::layer().json();
    #[cfg(feature = "telemetry")]
    let json_layer = json_layer.event_format(tracing_helper::TraceIdFormat);
    let json_layer = json_layer
        .with_writer(rabbit_digger_pro::log::LogWriter::new)
        .with_filter(dynamic_filter_fn(move |metadata, ctx| {
            log_writer_filter.enabled(metadata, ctx.clone())
        }));

    tr.with(
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stdout)
            .with_filter(dynamic_filter_fn(move |metadata, ctx| {
                log_filter.enabled(metadata, ctx.clone())
            })),
    )
    .with(json_layer)
    .init();

    match &args.cmd {
        Some(Command::GenerateSchema { path }) => {
            if let Some(path) = path {
                schema::write_schema(path).await?;
            } else {
                let s = schema::generate_schema().await?;
                println!("{}", serde_json::to_string(&s)?);
            }
            return Ok(());
        }
        Some(Command::Server { api_server }) => {
            let app = App::new().await?;

            app.run_api_server(api_server.to_api_server()).await?;

            tokio::signal::ctrl_c().await?;

            return Ok(());
        }
        None => {}
    }

    set_panic_hook();
    match real_main(args).await {
        Ok(()) => {}
        Err(e) => tracing::error!("Process exit: {:?}", e),
    }

    Ok(())
}

#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Exit the whole process when panic.
pub fn set_panic_hook() {
    use std::{panic, process};

    // HACK! New a backtrace ahead for caching necessary elf sections of this
    // tikv-server, in case it can not open more files during panicking
    // which leads to no stack info (0x5648bdfe4ff2 - <no info>).
    //
    // Crate backtrace caches debug info in a static variable `STATE`,
    // and the `STATE` lives forever once it has been created.
    // See more: https://github.com/alexcrichton/backtrace-rs/blob/\
    //           597ad44b131132f17ed76bf94ac489274dd16c7f/\
    //           src/symbolize/libbacktrace.rs#L126-L159
    // Caching is slow, spawn it in another thread to speed up.
    // thread::Builder::new()
    //     .name(thd_name!("backtrace-loader"))
    //     .spawn_wrapper(::backtrace::Backtrace::new)
    //     .unwrap();

    // let data_dir = data_dir.to_string();

    panic::set_hook(Box::new(move |info: &panic::PanicInfo<'_>| {
        // let msg = match info.payload().downcast_ref::<&'static str>() {
        //     Some(s) => *s,
        //     None => match info.payload().downcast_ref::<String>() {
        //         Some(s) => &s[..],
        //         None => "Box<Any>",
        //     },
        // };

        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()));
        let bt = std::backtrace::Backtrace::capture();
        eprintln!(
            "thread_name = {:?}, location = {:?}, backtrace = {:?}",
            name,
            loc.unwrap_or_else(|| "<unknown>".to_owned()),
            format_args!("{:?}", bt),
        );

        // There might be remaining logs in the async logger.
        // To collect remaining logs and also collect future logs, replace the old one
        // with a terminal logger.
        // When the old global async logger is replaced, the old async guard will be
        // taken and dropped. In the drop() the async guard, it waits for the
        // finish of the remaining logs in the async logger.
        // if let Some(level) = ::log::max_level().to_level() {
        //     let drainer = logger::text_format(logger::term_writer(), true);
        //     let _ = logger::init_log(
        //         drainer,
        //         logger::convert_log_level_to_slog_level(level),
        //         false, // Use sync logger to avoid an unnecessary log thread.
        //         false, // It is initialized already.
        //         vec![],
        //         0,
        //     );
        // }

        // If PANIC_MARK is true, create panic mark file.
        // if panic_mark_is_on() {
        //     create_panic_mark_file(data_dir.clone());
        // }

        process::abort();
        // if panic_abort {
        //     process::abort();
        // } else {
        //     unsafe {
        //         // Calling process::exit would trigger global static to destroy, like C++
        //         // static variables of RocksDB, which may cause other threads encounter
        //         // pure virtual method call. So calling libc::_exit() instead to skip the
        //         // cleanup process.
        //         libc::_exit(1);
        //     }
        // }
    }))
}
