extern crate mio;
extern crate http_muncher;
extern crate sha1;
extern crate rustc_serialize;
extern crate byteorder;

use std::net::SocketAddr;
use std::collections::HashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::fmt;

use mio::*;
use mio::tcp::*;
use rustc_serialize::base64::{ToBase64, STANDARD};
use http_muncher::{Parser, ParserHandler};

mod frame;

use frame::{WebSocketFrame, OpCode};

const SERVER_TOKEN: Token = Token(0);

fn gen_key(key: &String) -> String {
    let mut m = sha1::Sha1::new();
    let mut buf = [0u8; 20];

    m.update(key.as_bytes());
    m.update("258EAFA5-E914-47DA-95CA-C5AB0DC85B11".as_bytes());

    m.output(&mut buf);

    return buf.to_base64(STANDARD);
}

#[derive(PartialEq)]
enum ClientState {
    AwaitingHandshake(RefCell<Parser<HttpParser>>),
    HandshakeResponse,
    Connected,
}
struct WebSocketServer {
    socket: TcpListener,
    clients: HashMap<Token, WebSocketClient>,
    token_counter: usize,
}

impl Handler for WebSocketServer {
    //Traits can have useful default implementations, so in fact the handler
    //interface requres us to provide only two things: concrete types for
    //timeouts and messages.
    //We're not ready to cover these fancy details, and we wouldn't get to them
    //anytime soon, so let's get along with the defauls from the mio exampes:
    type Timeout = usize;
    type Message = ();

    fn ready(&mut self, event_loop: &mut EventLoop<WebSocketServer>,
             token: Token, events: EventSet)
    {
        if events.is_readable() {
            match token {
                SERVER_TOKEN => {
                    let client_socket = match self.socket.accept() {
                        Err(e) => {
                            println!("Accept error: {}", e);
                            return;
                        },
                        Ok(None) => unreachable!("Accept as returned 'None'"),
                        Ok(Some((sock, addr))) => sock,
                    };

                    let new_token = Token(self.token_counter);
                    self.clients.insert(new_token, WebSocketClient::new(client_socket));
                    self.token_counter +=1;

                    event_loop.register(&self.clients[&new_token].socket, 
                                        new_token, 
                                        EventSet::readable(),
                                        PollOpt::edge() | PollOpt::oneshot()).unwrap();
                },
                token => {
                    let mut client = self.clients.get_mut(&token).unwrap();
                    client.read();
                    event_loop.reregister(&client.socket, token, 
                                          client.interest, // Providing interest from the client struct
                                          PollOpt::edge() | PollOpt::oneshot()).unwrap();
                    println!("New Client?");
                }
            }
        }

        if events.is_writable() {
            let client = self.clients.get_mut(&token).unwrap();
            client.write();
            event_loop.reregister(&client.socket, token, client.interest, 
                                  PollOpt::edge() | PollOpt::oneshot()).unwrap();
        }
    }
}

struct HttpParser {
    current_key:    Option<String>,
    headers:        Rc<RefCell<HashMap<String, String>>>
}

impl ParserHandler for HttpParser {
    fn on_header_field(&mut self, s: &[u8]) -> bool {
        self.current_key = Some(std::str::from_utf8(s).unwrap().to_string());
        true
    }

    fn on_header_value(&mut self, s: &[u8]) -> bool {
        self.headers.borrow_mut()
            .insert(self.current_key.clone().unwrap(),
            std::str::from_utf8(s).unwrap().to_string());
        true
    }

    fn on_headers_complete(&mut self) -> bool {
        false
    }
}

struct WebSocketClient {
    socket: TcpStream,
    headers: Rc<RefCell<HashMap<String, String>>>,

    interest: EventSet,
    
    state: ClientState,
}

impl WebSocketClient {
    fn write(&mut self) {
        // Get the headers of HashMap from the Rc<RefCell<..>> wrapper:
        let headers = self.headers.borrow();

        // Find the header that interests us, and generate the key from its value:
        let response_key = gen_key(&headers.get("Sec-WebSocket-Key").unwrap());

        let response = fmt::format(format_args!("HTTP/1.1 101 Switching Protocols \r\n\
                                                Connection: Upgrade\r\n\
                                                Sec-WebSocket-Accept: {}\r\n\
                                                Upgrade: websocket\r\n\r\n", response_key));

        // Write the repsonse to the socket:
        self.socket.try_write(response.as_bytes()).unwrap();

        // Change the state:
        self.state = ClientState::Connected;

        // And change the interest back to `readable()`:
        self.interest.remove(EventSet::writable());
        self.interest.insert(EventSet::readable());
    }

    fn read_handshake(&mut self) {
        loop {
            let mut buf = [0; 2048];
            let is_upgrade = if let ClientState::AwaitingHandshake(ref parser_state) = self.state {
                let mut parser = parser_state.borrow_mut();
                parser.parse(&buf);
                parser.is_upgrade()
            } else { false };

            if is_upgrade {
                // Change the current state
                self.state = ClientState::HandshakeResponse;
                self.interest.remove(EventSet::readable());
                self.interest.remove(EventSet::writable());
            }
        }    
    }

    fn read(&mut self) {
        match self.state {
            ClientState::AwaitingHandshake(_) => {
                self.read_handshake();
            },
            ClientState::Connected => {
                let frame = WebSocketFrame::read(&mut self.socket);
                match frame {
                    Ok(frame) => println!("{:?}", frame),
                    Err(e)  => println!("error while reading frame: {}", e),
                }
            }
        }
    }

    fn new(socket: TcpStream) -> WebSocketClient {
        let headers = Rc::new(RefCell::new(HashMap::new()));

        WebSocketClient {
            socket: socket,

            // We're making a first clone of the `headers` variable
            // to read its contents
            headers: headers.clone(),

            interest: EventSet::readable(),

            state: ClientState::AwaitingHandshake(RefCell::new(Parser::request(HttpParser {
                current_key: None,
                headers: headers.clone()
            }))),
        }
    }
}


fn main() {
    let address = "0.0.0.0:10000".parse::<SocketAddr>().unwrap(); 
    let server_socket = TcpListener::bind(&address).unwrap();
    let mut event_loop = EventLoop::new().unwrap();
    // Create a new instance of our handler struct:
    let mut server = WebSocketServer {
        token_counter:  1,              // Starting the token counter from 1
        clients:        HashMap::new(), // create empty hashmap
        socket:         server_socket,  // Handling the ownership of the socket to the struct
    };

    event_loop.register(&server.socket,
                        SERVER_TOKEN,
                        EventSet::readable(),
                        PollOpt::edge()).unwrap();
    // ... and provide the even loop with a mutable reference to it:

    event_loop.run(&mut server).unwrap();
}
