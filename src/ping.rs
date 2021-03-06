use std::error::Error;
use std::fmt;
use std::sync::{Arc, Barrier, Mutex};
// TODO:
// Add PoisonErrs similar to IVar

// CONSIDER FOR DEADLOCK FREEDOM:
// Breaking into sender + reciever, killing a wait if the other drops from exisitence.

// This is a single-use rendezvous channel, obeying the laws of Pi Calculus.
pub struct Ping<T>(Arc<PingMachine<T>>);

impl<T> Clone for Ping<T> {
    fn clone(&self) -> Ping<T> {
        Ping::<T>(self.0.clone())
    }
}

#[derive(Debug)]
pub enum PingState {
    Unintialized, // Shouldn't happen, but who knows what someone will do.
    Open,         // Neither Send or Recieve is Used.
    AwaitSend,    // A listener is waiting.
    AwaitRecv,    // A sender is waiting.
    Used,         // A transfer was made.
}

impl fmt::Display for PingState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PingState::Unintialized => write!(f, "Uninitalized (ILLEGAL, USE Ping::<T>::new)"),
            PingState::Open => write!(f, "Open"),
            PingState::AwaitSend => write!(f, "AwaitSend"),
            PingState::AwaitRecv => write!(f, "AwaitRecv"),
            PingState::Used => write!(f, "Used"),
        }
    }
}

struct PingMachine<T> {
    init: Arc<Mutex<bool>>,
    val: Arc<Mutex<Option<T>>>,
    send_guard: Arc<Mutex<bool>>,
    send_bar: Arc<Barrier>,
    recv_guard: Arc<Mutex<bool>>,
    recv_bar: Arc<Barrier>,
}

impl<T> Ping<T> {
    pub fn new() -> Ping<T> {
        let send_barrier = Arc::new(Barrier::new(2));
        let recv_barrier = Arc::new(Barrier::new(2));
        // Each thread will decrement each barrier once.
        // The recv barrier is lifted by both parties.
        // The send barrier is lifted by the reciever.
        // The sender places the value.
        // The send barrier is lifted by the sender.
        // The reciever extracts the value.

        // If this were not rendezvous or were long lived
        // WaitGroups would be needed to "lock the door" behind you.

        Ping::<T>(Arc::new(PingMachine::<T> {
            init: Arc::new(Mutex::new(true)),
            val: Arc::new(Mutex::new(None)),
            send_guard: Arc::new(Mutex::new(false)),
            send_bar: send_barrier,
            recv_guard: Arc::new(Mutex::new(false)),
            recv_bar: recv_barrier,
        }))
    }

    pub fn state(&self) -> PingState {
        match self.check_init() {
            false => PingState::Unintialized,
            _ => match self.check_send_used() {
                true => match self.check_recv_used() {
                    true => PingState::Used,
                    _ => PingState::AwaitRecv,
                },
                _ => match self.check_recv_used() {
                    true => PingState::AwaitSend,
                    _ => PingState::Open,
                },
            },
        }
    }

    pub fn send(&mut self, t: T) -> Result<(), PingError> {
        match self.check_init() {
            // We have come into the possession of an uninitialized channel through spectacular means.
            false => Err(PingError::UninitializedChanError),
            true => {
                let r = self.set_send_used();

                match r {
                    // We are not the winning sender, the channel has been used.
                    Err(x) => Err(x),

                    Ok(()) => {
                        // Detect Recieve.
                        self.0.recv_bar.wait();

                        // finally.
                        let mut data = self.0.val.lock().unwrap();
                        *data = Some(t);

                        // Inform recieve we exist
                        self.0.send_bar.wait();

                        // Weaken references to self?
                        // If so, one here.
                        Ok(())
                    }
                }
            }
        }
    }

    pub fn recv(&mut self) -> Result<T, PingError> {
        match self.check_init() {
            // We have come into the possession of an uninitialized channel through spectacular means.
            false => Err(PingError::UninitializedChanError),
            true => {
                let r = self.set_recv_used();
                match r {
                    Err(x) => Err(x),
                    Ok(()) => {
                        self.0.recv_bar.wait(); // Alert the sender.

                        self.0.send_bar.wait(); // Await the sender.

                        Ok(self.0.val.lock().unwrap().take().unwrap())
                    }
                }
            }
        }
    }

    fn set_send_used(&mut self) -> Result<(), PingError> {
        let mut is_used = self.0.send_guard.lock().unwrap();

        match *is_used {
            true => Err(PingError::UsedSendChanError),
            false => {
                *is_used = true;
                Ok(())
            }
        }
    }

    fn set_recv_used(&mut self) -> Result<(), PingError> {
        let mut is_used = self.0.recv_guard.lock().unwrap();

        match *is_used {
            true => Err(PingError::UsedRecvChanError),
            false => {
                *is_used = true;
                Ok(())
            }
        }
    }

    fn check_send_used(&self) -> bool {
        *self.0.send_guard.lock().unwrap()
    }

    fn check_recv_used(&self) -> bool {
        *self.0.recv_guard.lock().unwrap()
    }

    fn check_init(&self) -> bool {
        *self.0.init.lock().unwrap()
    }
}

#[derive(Debug)]
pub enum PingError {
    UsedSendChanError,
    UsedRecvChanError,
    UninitializedChanError,
}

impl fmt::Display for PingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PingError::UsedSendChanError => write!(f, "This instance of Ping already has a sender"),
            PingError::UsedRecvChanError => {
                write!(f, "This instance of Ping already has a reciever")
            }
            PingError::UninitializedChanError => {
                write!(f, "Ping must be initialized to use safely")
            }
        }
    }
}

impl Error for PingError {
    fn description(&self) -> &str {
        match self {
            PingError::UsedSendChanError => "This instance of Ping already has a sender",
            PingError::UsedRecvChanError => "This instance of Ping already has a reciever",
            PingError::UninitializedChanError => "Ping must be initialized to use safely",
        }
    }

    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

pub struct Spark<T> (Ping<T>);
impl<T> Spark<T> {
    pub fn read(&mut self) -> Result<T, PingError> {
        self.0.recv()
    }
}

pub fn spark<T: 'static, U: 'static>(arg: T, action: Box<dyn FnOnce(T) -> U + Send>) -> Spark<U>
where
    T: Send,
    U: Send,
{
    let p = Ping::<U>::new();
    let mut q = p.clone();
    let f = move || {
        let x = action(arg);
        let r = q.send(x);
        // Given that Ping is a rendezvous, this will block until the caller lifts the barriers.
        // Consider creating non-rendezvous channel and replacing later.
        match r {
            Ok(_) => {},
            Err(err) => {
                println!("Impossible state in spark! Err in interal send {}", err)
            }
        }
    };

    std::thread::spawn(f);

    Spark::<U>(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_ping() {
        let mut p1 = Ping::<bool>::new();
        let mut q1 = p1.clone();

        let un_init = p1.state();
        match un_init {
            PingState::Open => println!(""),
            _ => panic!("P1 was in unexpected state! {}", un_init),
        };

        let mut p2 = Ping::<bool>::new();
        let mut q2 = p2.clone();

        let h = thread::spawn(move || {
            let non_determ = q1.state();
            match non_determ {
                PingState::Open => println!("Q1 is open, we are ahead of the main thread"),
                PingState::AwaitRecv => {
                    println!("Q1 is awaiting a reciever, we are behind the main thread")
                }
                _ => panic!("Q1 is in an unexpected state!, {}", non_determ),
            };

            let q1_result = q1.recv();
            match q1_result {
                Ok(y) => {
                    assert!(y);
                }
                Err(err) => {
                    panic!("Thread 2: Heard Err Listening to c1 {}", err);
                }
            };

            let used = q1.state();
            match used {
                PingState::Used => println!("Q1 is used as expected"),
                _ => panic!("Q1 was in unexpected state! {}", used),
            };

            let non_determ = q2.state();
            match non_determ {
                PingState::Open => println!("Q2 is open, we are ahead of the main thread"),
                PingState::AwaitSend => {
                    println!("Q2 is awaiting a sender, we are behind the main thread")
                }
                _ => panic!("Q2 is in an unexpected state!, {}", non_determ),
            };

            q2.send(true).expect("Send on used channel for c2?");

            let used = q2.state();
            match used {
                PingState::Used => println!("Q2 is used as expected"),
                _ => panic!("Q2 was in unexpected state! {}", used),
            };
        });

        p1.send(true).expect("Send on used channel for p1");
        let p2_result = p2.recv();
        match p2_result {
            Ok(y) => {
                assert!(y);
            }
            Err(err) => {
                panic!("Thread 2: Heard Err Listening to c1 {}", err);
            }
        }

        h.join().expect("Failed to Join Threads!");

        let err1 = p1.send(true);
        match err1 {
            Err(_) => println!(""),
            Ok(_) => panic!("Send allowed on closed channel"),
        }

        let err2 = p2.recv();
        match err2 {
            Err(_) => println!(""),
            Ok(_) => panic!("Recv allowed on closed channel"),
        }
    }

    #[test]
    fn test_spark(){
        let f = |i: i32| i * i;
        let mut my_spark = spark(4, Box::new(f));
        let result = my_spark.read();
        match result {
            Ok(x) => {
                assert_eq!(16, x)
            }
            _ => panic!("No result")
        }
    }
}
