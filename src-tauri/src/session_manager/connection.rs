use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use russh::client::{Handle, Msg};
use russh::Error::Disconnect;
use russh::{Channel, ChannelMsg};
use tauri::{AppHandle, Runtime};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};
use uuid::Uuid;
use vt100::Parser;

use crate::device_manager::Device;
use crate::error::Error;
use crate::session_manager::handler::ClientHandler;
use crate::session_manager::{Proc, Shell, ShellToken};

pub(crate) struct Connection {
    pub(crate) id: Uuid,
    pub(crate) device: Device,
    pub(crate) handle: AsyncMutex<Handle<ClientHandler>>,
    pub(crate) connections: Weak<Mutex<ConnectionsMap>>,
}

pub(crate) type ConnectionsMap = HashMap<String, Arc<Connection>>;

impl Connection {
    pub async fn exec(&self, command: &str, stdin: Option<&[u8]>) -> Result<Vec<u8>, Error> {
        let mut ch = self.open_cmd_channel().await?;
        let id = ch.id();
        log::debug!("{id}: Exec {{ command: {command} }}");
        ch.exec(true, command).await?;
        if !Connection::wait_reply(&mut ch).await? {
            return Err(Error::NegativeReply);
        }
        if let Some(data) = stdin {
            ch.data(data).await?;
            ch.eof().await?;
        }
        let mut stdout: Vec<u8> = Vec::new();
        let mut stderr: Vec<u8> = Vec::new();
        let mut status: Option<u32> = None;
        let mut eof: bool = false;
        loop {
            match ch.wait().await.ok_or(Error::new("empty message"))? {
                ChannelMsg::Data { data } => {
                    log::trace!("{id}: Data {{ data: {data:?} }}");
                    stdout.append(&mut data.to_vec());
                }
                ChannelMsg::ExtendedData { data, ext } => {
                    log::trace!("{id}: ExtendedData {{ data: {data:?}, ext: {ext} }}");
                    if ext == 1 {
                        stderr.append(&mut data.to_vec());
                    }
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    log::debug!("{id}: ExitStatus {{ exit_status: {exit_status} }}");
                    status = Some(exit_status);
                    if eof {
                        break;
                    }
                }
                ChannelMsg::Eof => {
                    log::debug!("{id}: Eof");
                    eof = true;
                    if status.is_some() {
                        break;
                    }
                }
                msg => log::debug!("{id}: {msg:?}"),
            }
        }
        let status = status.unwrap_or(0);
        if status != 0 {
            return Err(Error::ExitStatus {
                message: format!("Command `{}` exited with status {}", command, status),
                exit_code: status,
                stderr,
            });
        }
        return Ok(stdout.to_vec());
    }

    pub async fn spawn(&self, command: &str) -> Result<Proc, Error> {
        let ch = self.open_cmd_channel().await?;
        let id = ch.id();
        log::debug!("{id}: Spawn {{ command: {command} }}");
        return Ok(Proc {
            command: String::from(command),
            ch: AsyncMutex::new(Some(ch)),
            sender: Mutex::default(),
            semaphore: Semaphore::new(0),
            callback: Mutex::default(),
        });
    }

    pub async fn shell(&self, cols: u16, rows: u16, dumb: Option<bool>) -> Result<Shell, Error> {
        let mut ch = self.open_cmd_channel().await?;
        let mut got_pty = false;
        if !dumb.unwrap_or(false) {
            ch.request_pty(true, "xterm", cols as u32, rows as u32, 0, 0, &[])
                .await?;
            got_pty = Connection::wait_reply(&mut ch).await?;
        }
        ch.request_shell(true).await?;
        if !Connection::wait_reply(&mut ch).await? {
            return Err(Error::NegativeReply);
        }
        return Ok(Shell {
            token: ShellToken {
                connection_id: self.id,
                channel_id: ch.id().to_string(),
            },
            created_at: Instant::now(),
            def_title: format!("{}@{}", self.device.username, self.device.name),
            has_pty: got_pty,
            channel: AsyncMutex::new(Some(ch)),
            sender: Mutex::default(),
            callback: Mutex::default(),
            parser: Mutex::new(Parser::new(rows, cols, 1000)),
        });
    }

    pub(crate) fn remove_from_pool(&self) -> () {
        if let Some(connections) = self.connections.upgrade() {
            connections.lock().unwrap().remove(&self.device.name);
        }
    }

    async fn open_cmd_channel(&self) -> Result<Channel<Msg>, Error> {
        let result = self.handle.lock().await.channel_open_session().await;
        if let Err(e) = result {
            self.remove_from_pool();
            if let russh::Error::ChannelOpenFailure(_) = e {
                return Err(Error::NeedsReconnect);
            }
            return Err(e.into());
        }
        return Ok(result?);
    }

    pub(crate) fn new(
        id: Uuid,
        device: Device,
        handle: Handle<ClientHandler>,
        connections: Weak<Mutex<ConnectionsMap>>,
    ) -> Connection {
        log::info!("Created connection {} for device {}", id, device.name);
        return Connection {
            id,
            device,
            handle: AsyncMutex::new(handle),
            connections,
        };
    }

    pub(crate) async fn wait_reply(ch: &mut Channel<Msg>) -> Result<bool, russh::Error> {
        loop {
            match ch.wait().await.ok_or_else(|| russh::Error::SendError)? {
                ChannelMsg::Success => return Ok(true),
                ChannelMsg::Failure => return Ok(false),
                ChannelMsg::WindowAdjusted { .. } => continue,
                e => {
                    log::warn!("unknown message waiting for channel {:?}: {:?}", ch.id(), e);
                    continue;
                }
            }
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        log::info!(
            "Dropped connection {} for device {}",
            self.id,
            self.device.name
        );
    }
}
