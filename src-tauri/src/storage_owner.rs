//! A single owner executes mutations; callers exchange commands rather than share state.
use std::sync::mpsc;

use crate::resources::ResourceError;

type Command<S> = Box<dyn FnOnce(&mut S) + Send>;

pub(crate) struct Owner<S> {
    sender: mpsc::Sender<Command<S>>,
}

impl<S: Send + 'static> Owner<S> {
    pub(crate) fn new(name: &str, mut state: S) -> Result<Self, ResourceError> {
        let (sender, receiver) = mpsc::channel::<Command<S>>();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                for command in receiver {
                    command(&mut state);
                }
            })
            .map_err(|error| ResourceError::new(error.to_string()))?;
        Ok(Self { sender })
    }

    pub(crate) fn call<T: Send + 'static>(
        &self,
        command: impl FnOnce(&mut S) -> Result<T, ResourceError> + Send + 'static,
    ) -> Result<T, ResourceError> {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.sender
            .send(Box::new(move |state| {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| command(state)))
                        .unwrap_or_else(|_| {
                            Err(ResourceError::new(
                                "Storage command panicked and was rolled back",
                            ))
                        });
                let _ = sender.send(result);
            }))
            .map_err(|_| ResourceError::new("Storage owner stopped"))?;
        receiver
            .recv()
            .map_err(|_| ResourceError::new("Storage owner stopped before replying"))?
    }
}
