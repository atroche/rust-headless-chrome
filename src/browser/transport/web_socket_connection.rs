use std::sync::mpsc;

use failure::Error;
use log::*;

use websocket::client::sync::Client;
use websocket::stream::sync::TcpStream;
use websocket::WebSocketError;
use websocket::{ClientBuilder, OwnedMessage};

use crate::protocol;
use std::sync::Mutex;

pub struct WebSocketConnection {
    sender: Mutex<websocket::sender::Writer<TcpStream>>,
    process_id: u32,
}

// TODO websocket::sender::Writer is not :Debug...
impl std::fmt::Debug for WebSocketConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        write!(f, "WebSocketConnection {{}}")
    }
}

impl WebSocketConnection {
    pub fn new(
        ws_url: &str,
        process_id: u32,
        messages_tx: mpsc::Sender<protocol::Message>,
    ) -> Result<Self, Error> {
        let connection = Self::websocket_connection(&ws_url)?;
        let (websocket_receiver, sender) = connection.split()?;

        std::thread::spawn(move || {
            trace!("Starting msg dispatching loop");
            Self::dispatch_incoming_messages(websocket_receiver, messages_tx, process_id);
            trace!("Quit loop msg dispatching loop");
        });

        Ok(Self {
            sender: Mutex::new(sender),
            process_id,
        })
    }

    pub fn shutdown(&self) {
        trace!(
            "Shutting down WebSocket connection for Chrome {}",
            self.process_id
        );
        if self.sender.lock().unwrap().shutdown_all().is_err() {
            warn!(
                "Couldn't shut down WS connection for Chrome {}",
                self.process_id
            );
        }
    }

    fn dispatch_incoming_messages(
        mut receiver: websocket::receiver::Reader<TcpStream>,
        messages_tx: mpsc::Sender<protocol::Message>,
        process_id: u32,
    ) {
        for ws_message in receiver.incoming_messages() {
            match ws_message {
                Err(error) => match error {
                    WebSocketError::NoDataAvailable => {
                        warn!("WS Error Chrome #{}: {}", process_id, error);
                        break;
                    }
                    WebSocketError::IoError(err) => {
                        warn!("WS IO Error for Chrome #{}: {}", process_id, err);
                        break;
                    }
                    _ => panic!(
                        "Unhandled WebSocket error for Chrome #{}: {:?}",
                        process_id, error
                    ),
                },
                Ok(message) => {
                    if let OwnedMessage::Text(message_string) = message {
                        if let Ok(message) = protocol::parse_raw_message(&message_string) {
                            if messages_tx.send(message).is_err() {
                                break;
                            }
                        } else {
                            trace!(
                                "Incoming message isn't recognised as event or method response: {}",
                                message_string
                            );
                        }
                    } else {
                        panic!("Got a weird message: {:?}", message)
                    }
                }
            }
        }

        info!("Sending shutdown message to message handling loop");
        if messages_tx
            .send(protocol::Message::ConnectionShutdown)
            .is_err()
        {
            warn!("Couldn't send message to transport loop telling it to shut down")
        }
    }

    pub fn websocket_connection(ws_url: &str) -> Result<Client<TcpStream>, Error> {
        let client = ClientBuilder::new(ws_url)?.connect_insecure()?;

        debug!("Successfully connected to WebSocket: {}", ws_url);

        Ok(client)
    }

    pub fn send_message(&self, message_text: &str) -> Result<(), Error> {
        let message = websocket::Message::text(message_text);
        let mut sender = self.sender.lock().unwrap();
        sender.send_message(&message)?;
        Ok(())
    }
}

