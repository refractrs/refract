use super::events;
use crate::service::StreamEvent;
use refract::{Cleanup, MessageSender, SendError};
use std::{sync::mpsc, time::Duration};

pub(crate) fn launch(
    prompt: String,
    sender: MessageSender<StreamEvent>,
) -> Result<Cleanup, String> {
    let (cancel, cancelled) = mpsc::channel::<()>();
    std::thread::Builder::new()
        .name("chat-reply".into())
        .spawn(move || {
            // Generation and pacing happen on the worker, never on the UI thread.
            for mut event in events(&prompt) {
                if cancelled.recv_timeout(Duration::from_millis(65))
                    != Err(mpsc::RecvTimeoutError::Timeout)
                {
                    return;
                }
                loop {
                    match sender.try_send(event) {
                        Ok(()) => break,
                        Err(SendError::Closed(_)) => return,
                        Err(SendError::Full(pending)) => event = pending,
                    }
                    // Retain the unsent chunk, including completion, under backpressure.
                    if cancelled.recv_timeout(Duration::from_millis(10))
                        != Err(mpsc::RecvTimeoutError::Timeout)
                    {
                        return;
                    }
                }
            }
        })
        .map_err(|error| error.to_string())?;
    // Disconnect wakes a sleeping worker immediately; never join it on the UI thread.
    Ok(Box::new(move || drop(cancel)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use refract::Runtime;
    use std::{cell::RefCell, rc::Rc, time::Instant};

    #[test]
    fn real_worker_retries_a_full_inbox_and_delivers_every_chunk() {
        let mut rt = Runtime::default();
        let received = Rc::new(RefCell::new((String::new(), 0)));
        let output = received.clone();
        let (sender, _subscription) = rt.channel(move |_, event| match event {
            StreamEvent::Chunk { text, complete } => {
                let mut output = output.borrow_mut();
                output.0.push_str(&text);
                output.1 += usize::from(complete);
            }
            StreamEvent::Failed(error) => panic!("unexpected worker failure: {error}"),
        });
        let (filler, _filler_subscription) = rt.channel(|_, _: ()| {});
        while filler.try_send(()).is_ok() {}
        let cancel = launch("hello".into(), sender).unwrap();
        // Let the first chunk encounter backpressure before making room.
        std::thread::sleep(Duration::from_millis(120));
        assert!(received.borrow().0.is_empty());
        let deadline = Instant::now() + Duration::from_secs(8);
        while received.borrow().1 == 0 && Instant::now() < deadline {
            rt.drain_messages();
            std::thread::sleep(Duration::from_millis(10));
        }
        cancel();
        assert_eq!(received.borrow().0, crate::service::reply("hello"));
        assert_eq!(
            received.borrow().1,
            1,
            "completion must arrive exactly once"
        );
    }
}
