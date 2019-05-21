extern crate mio;
use mio::*;
use mio::tcp::*;
use std::net::SocketAddr;

struct WebSocketServer;

impl Handler for WebSocketServer {
    //Traits can have useful default implementations, so in fact the handler
    //interface requres us to provide only two things: concrete types for
    //timeouts and messages.
    //We're not ready to cover these fancy details, and we wouldn't get to them
    //anytime soon, so let's get along with the defauls from the mio exampes:
    type Timeout = usize;
    type Message = ();
}
fn main() {
    let address = "0.0.0.0:10000".parse::<SocketAddr>().unwrap(); 
    let server_socket = TcpListener::bind(&address).unwrap();
    let mut event_loop = EventLoop::new().unwrap();
    // Create a new instance of our handler struct:
    let mut handler = WebSocketServer;

    event_loop.register(&server_socket,
                        Token(0),
                        EventSet::readable(),
                        PollOpt::edge()).unwrap();
    // ... and provide the even loop with a mutable reference to it:
    event_loop.run(&mut handler).unwrap();
}
