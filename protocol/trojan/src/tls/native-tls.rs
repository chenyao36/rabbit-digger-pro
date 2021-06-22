use super::TlsConnectorConfig;
use rd_interface::{error::map_other, AsyncRead, AsyncWrite, Result};
use tokio_native_tls::native_tls;

pub use tokio_native_tls::TlsStream;

pub struct TlsConnector {
    sni: String,
    connector: tokio_native_tls::TlsConnector,
}

impl TlsConnector {
    pub fn new(config: TlsConnectorConfig) -> Result<TlsConnector> {
        let mut builder = native_tls::TlsConnector::builder();
        if config.skip_cert_verify {
            builder.danger_accept_invalid_certs(true);
        }
        let connector = tokio_native_tls::TlsConnector::from(builder.build().map_err(map_other)?);

        Ok(TlsConnector {
            sni: config.sni,
            connector,
        })
    }
    pub async fn connect<IO>(&self, stream: IO) -> Result<TlsStream<IO>>
    where
        IO: AsyncRead + AsyncWrite + Unpin,
    {
        let stream = self
            .connector
            .connect(self.sni.as_ref(), stream)
            .await
            .map_err(map_other)?;
        Ok(stream)
    }
}
