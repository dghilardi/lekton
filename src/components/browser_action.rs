//! Actions whose futures only ever run in the browser.
use leptos::prelude::*;
use std::future::Future;

/// Creates an action for a `!Send` future, such as one going through
/// `with_auth_retry`.
///
/// `Action::new_local` keeps its closure in a `SendWrapper`. Created while a
/// page renders on the server, it is dropped by the response owner's cleanup
/// on whichever worker thread finishes the response; dropping it from another
/// thread panics, and a second panic during that cleanup aborts the process.
/// Server-rendered actions are never dispatched, so the server gets an inert
/// action instead and the browser keeps `Action::new_local`.
#[track_caller]
pub fn browser_action<I, O, F, Fu>(action_fn: F) -> Action<I, O>
where
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    F: Fn(&I) -> Fu + 'static,
    Fu: Future<Output = O> + 'static,
{
    #[cfg(feature = "hydrate")]
    {
        Action::new_local(action_fn)
    }
    #[cfg(not(feature = "hydrate"))]
    {
        drop(action_fn);
        Action::new(|_: &I| std::future::pending::<O>())
    }
}

#[cfg(all(test, not(feature = "hydrate")))]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[test]
    fn server_actions_can_be_cleaned_up_on_another_thread() {
        let owner = Owner::new();
        owner.with(|| {
            // An `Rc` makes the closure !Send, as with the auth retry futures.
            let local = Rc::new(());
            let _action = browser_action(move |_: &()| {
                let local = Rc::clone(&local);
                async move { drop(local) }
            });
        });
        let cleanup = std::thread::spawn(move || owner.cleanup());
        assert!(
            cleanup.join().is_ok(),
            "cleaning up on another thread panicked"
        );
    }

    #[test]
    #[should_panic(expected = "cleaning up on another thread panicked")]
    fn local_actions_cannot_be_cleaned_up_on_another_thread() {
        // Documents the failure this helper avoids during SSR.
        let owner = Owner::new();
        owner.with(|| {
            let local = Rc::new(());
            let _action = Action::new_local(move |_: &()| {
                let local = Rc::clone(&local);
                async move { drop(local) }
            });
        });
        let cleanup = std::thread::spawn(move || owner.cleanup());
        assert!(
            cleanup.join().is_ok(),
            "cleaning up on another thread panicked"
        );
    }
}
