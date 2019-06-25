extern crate mio;
extern crate http_muncher;
extern crate sha1;
extern crate rustc_serialize;

use mio::*;
use mio::tcp::*;
use std::net::SocketAddr;
use std::collections::HashMap;
use rustc_serialize::base64::{ToBase64, STANDARD};
use std::cell::RefCell;
use std::rc::Rc;
use std::fmt;
use http_muncher::{Parser, ParserHandler};

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
    AwaitingHandshake,
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
    http_parser: Parser<HttpParser>,

    // Adding headers declaration to WebSocketClient struct
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

    fn read(&mut self) {
        loop {
            let mut buf = [0; 2048];

            match self.socket.try_read(&mut buf) {
                Err(e) => {
                    println!("Error while reading socket: {:?}", e);
                    return
                },
                Ok(None) =>
                    // Socket buffer has got no more bytes.
                    break,
                Ok(Some(len)) => {
                    self.http_parser.parse(&buf);
                    if self.http_parser.is_upgrade() {
                        // Change the current state
                        self.state = ClientState::HandshakeResponse;

                        // Change current interest to 'Writable'
                        self.interest.remove(EventSet::readable());
                        self.interest.insert(EventSet::writable());

                        break;
                    }
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

            http_parser: Parser::request(HttpParser {
                current_key: None,

                // and the second clone to write new headers to:
                headers: headers.clone(),
            }),

            interest: EventSet::readable(),

            state: ClientState::AwaitingHandshake,
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
