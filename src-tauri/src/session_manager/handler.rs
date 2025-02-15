use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;
use russh::{client, Error};
use russh_keys::key::{PublicKey, SignatureHash};
use uuid::Uuid;

use crate::session_manager::connection::ConnectionsMap;
use crate::session_manager::shell::ShellsMap;

#[derive(Default)]
pub(crate) struct ClientHandler {
    pub(super) id: Uuid,
    pub(super) key: String,
    pub(super) connections: Weak<Mutex<ConnectionsMap>>,
    pub(super) shells: Weak<Mutex<ShellsMap>>,
    pub(super) sig_alg: Arc<Mutex<Option<SignatureHash>>>,
}

#[async_trait]
impl client::Handler for ClientHandler {
    type Error = Error;

    async fn check_server_key(
        self,
        server_public_key: &PublicKey,
    ) -> Result<(Self, bool), Self::Error> {
        let alg: Option<SignatureHash> = match server_public_key {
            PublicKey::Ed25519(_) => None,
            PublicKey::RSA { .. } => {
                SignatureHash::from_rsa_hostkey_algo(server_public_key.name().as_bytes())
            }
        };
        *self.sig_alg.lock().unwrap() = alg;
        return Ok((self, true));
    }
}

impl Drop for ClientHandler {
    fn drop(&mut self) {
        if let Some(c) = self.connections.upgrade() {
            if let Some(removed) = c.lock().unwrap().remove(&self.key) {
                log::info!("Dropped connection to {}", removed.device.name)
            }
        }
    }
}
