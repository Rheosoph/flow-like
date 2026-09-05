//! Retain Rust objects between nodes in one package instance and run.
//!
//! Pass the string returned by [`insert`] through a String pin. The object stays
//! in guest memory and needs no serialization, `Send`, or `Sync` implementation.
//! Access is synchronous and checked by Rust type. Callbacks may access other
//! objects; conflicting access to the same object returns [`ResourceError::Borrowed`].
//!
//! This registry requires a reusable node export. Command-style components get
//! fresh guest memory for each command. The runtime releases guest memory and
//! WASI resources at run end, without executing guest destructors. Use [`remove`]
//! or [`close`] before returning when an object needs graceful cleanup. Keeping
//! an object here does not drive its event loop between node calls.
//!
//! On native targets the registry is local to the current thread for SDK tests.
//! Native tests should close their objects; there is no native flow-run owner.
//!
//! ```
//! use flow_like_wasm_sdk::resources;
//!
//! # fn example() -> Result<(), resources::ResourceError> {
//! // Node A creates an object and returns its handle through an output pin.
//! let handle = resources::insert(Vec::<String>::new())?;
//! // Node B receives the handle through an input pin.
//! resources::with_mut::<Vec<String>, _>(&handle, |parts| {
//!     parts.push("hello".into());
//! })?;
//! let text = resources::with::<Vec<String>, _>(&handle, |parts| parts.join(" "))?;
//! assert_eq!(text, "hello");
//! // A final node can take ownership for custom cleanup, or drop it with close.
//! resources::close::<Vec<String>>(&handle)?;
//! # Ok(())
//! # }
//! # example().unwrap();
//! ```

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

type Object = Rc<RefCell<Box<dyn Any>>>;

thread_local! {
    // A Wasm instance owns its thread-local storage. Package calls are serialized
    // by the host; this table is never shared with another package or run.
    static OBJECTS: RefCell<HashMap<String, Object>> = RefCell::new(HashMap::new());
}

/// An object handle could not be created or accessed in this package instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceError {
    /// The host cannot issue a handle, or it returned an existing handle.
    Unavailable,
    /// The handle was removed, belongs to another instance, or never existed.
    NotFound,
    /// The stored object's concrete Rust type differs from the requested type.
    TypeMismatch,
    /// A callback is already borrowing this object incompatibly with the operation.
    Borrowed,
}

impl fmt::Display for ResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "Cannot create an object handle in this execution",
            Self::NotFound => "Object handle is unavailable in this package instance and run",
            Self::TypeMismatch => "Object handle has a different Rust type",
            Self::Borrowed => "Object is already borrowed by another resource callback",
        })
    }
}

impl std::error::Error for ResourceError {}

/// Store an owned object and return an opaque handle suitable for a String pin.
///
/// The host issues fresh random handles, including for new instances of the same
/// package. Saving a handle does not preserve the object or make it usable in a
/// later run. Objects may own sockets supported by the guest's imports; this API
/// grants no additional network permissions or operating-system interfaces.
pub fn insert<T: 'static>(value: T) -> Result<String, ResourceError> {
    let handle = crate::host::new_resource_handle().ok_or(ResourceError::Unavailable)?;
    // Check before moving the value so an error drops it outside a registry borrow.
    if handle.is_empty() || OBJECTS.with(|objects| objects.borrow().contains_key(&handle)) {
        return Err(ResourceError::Unavailable);
    }
    OBJECTS.with(|objects| {
        objects
            .borrow_mut()
            .insert(handle.clone(), Rc::new(RefCell::new(Box::new(value))));
    });
    Ok(handle)
}

fn lookup(handle: &str) -> Result<Object, ResourceError> {
    OBJECTS.with(|objects| {
        objects
            .borrow()
            .get(handle)
            .cloned()
            .ok_or(ResourceError::NotFound)
    })
}

/// Read an object within a callback. The reference cannot escape the callback.
pub fn with<T: 'static, R>(handle: &str, access: impl FnOnce(&T) -> R) -> Result<R, ResourceError> {
    let object = lookup(handle)?;
    let value = object.try_borrow().map_err(|_| ResourceError::Borrowed)?;
    let value = value
        .downcast_ref::<T>()
        .ok_or(ResourceError::TypeMismatch)?;
    Ok(access(value))
}

/// Mutate an object within a callback. Other objects remain accessible.
///
/// Accessing or removing this object again while the callback is running returns
/// [`ResourceError::Borrowed`]. Finish the callback before awaiting asynchronous work.
pub fn with_mut<T: 'static, R>(
    handle: &str,
    access: impl FnOnce(&mut T) -> R,
) -> Result<R, ResourceError> {
    let object = lookup(handle)?;
    let mut value = object
        .try_borrow_mut()
        .map_err(|_| ResourceError::Borrowed)?;
    let value = value
        .downcast_mut::<T>()
        .ok_or(ResourceError::TypeMismatch)?;
    Ok(access(value))
}

/// Remove an object and return ownership, invalidating its handle immediately.
///
/// A type mismatch or active borrow leaves the object in the registry. The caller
/// can invoke protocol-specific shutdown on the returned object before dropping it.
pub fn remove<T: 'static>(handle: &str) -> Result<T, ResourceError> {
    let object = lookup(handle)?;
    {
        let value = object
            .try_borrow_mut()
            .map_err(|_| ResourceError::Borrowed)?;
        if !value.is::<T>() {
            return Err(ResourceError::TypeMismatch);
        }
    }
    OBJECTS.with(|objects| objects.borrow_mut().remove(handle));
    // No user code runs between the borrow check and removal. Any callback
    // holding another reference would have made try_borrow_mut fail above.
    let object = Rc::try_unwrap(object)
        .unwrap_or_else(|_| unreachable!("removed object has no active callbacks"));
    let value = object
        .into_inner()
        .downcast::<T>()
        .unwrap_or_else(|_| unreachable!("object type checked before removal"));
    Ok(*value)
}

/// Invalidate the handle and drop its object, running its Rust destructor now.
///
/// Destructors run outside registry borrows and may close other stored objects.
pub fn close<T: 'static>(handle: &str) -> Result<(), ResourceError> {
    drop(remove::<T>(handle)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    // Rc<Cell<_>> is neither Send nor Sync and has no serialization requirement.
    struct Session {
        parts: Vec<String>,
        dropped: Rc<Cell<usize>>,
    }

    impl Drop for Session {
        fn drop(&mut self) {
            self.dropped.set(self.dropped.get() + 1);
        }
    }

    #[test]
    fn arbitrary_objects_mutate_across_calls_and_drop_only_when_closed() {
        let dropped = Rc::new(Cell::new(0));
        let handle = insert(Session {
            parts: vec!["first".into()],
            dropped: dropped.clone(),
        })
        .unwrap();
        with_mut::<Session, _>(&handle, |session| session.parts.push("second".into())).unwrap();
        assert_eq!(
            with::<Session, _>(&handle, |session| session.parts.join(" ")).unwrap(),
            "first second"
        );
        assert_eq!(dropped.get(), 0);
        close::<Session>(&handle).unwrap();
        assert_eq!(dropped.get(), 1);
        assert_eq!(close::<Session>(&handle), Err(ResourceError::NotFound));
        let replacement = insert(42u32).unwrap();
        assert_ne!(handle, replacement);
        assert_eq!(
            with::<u32, _>(&handle, |n| *n),
            Err(ResourceError::NotFound)
        );
        close::<u32>(&replacement).unwrap();
    }

    #[test]
    fn wrong_types_and_unknown_handles_do_not_remove_or_mutate_objects() {
        let handle = insert(String::from("retained")).unwrap();
        assert_eq!(
            with::<u64, _>(&handle, |n| *n),
            Err(ResourceError::TypeMismatch)
        );
        assert_eq!(
            with_mut::<u64, _>(&handle, |n| *n = 0),
            Err(ResourceError::TypeMismatch)
        );
        assert_eq!(remove::<u64>(&handle), Err(ResourceError::TypeMismatch));
        assert_eq!(close::<u64>(&handle), Err(ResourceError::TypeMismatch));
        assert_eq!(remove::<String>("forged"), Err(ResourceError::NotFound));
        assert_eq!(remove::<String>(&handle).unwrap(), "retained");
    }

    #[test]
    fn nested_callbacks_allow_other_objects_and_reject_conflicting_borrows() {
        let first = insert(1u32).unwrap();
        let second = insert(2u32).unwrap();
        with_mut::<u32, _>(&first, |a| {
            assert_eq!(with::<u32, _>(&first, |n| *n), Err(ResourceError::Borrowed));
            assert_eq!(
                with_mut::<u32, _>(&first, |_| ()),
                Err(ResourceError::Borrowed)
            );
            assert_eq!(remove::<u32>(&first), Err(ResourceError::Borrowed));
            with_mut::<u32, _>(&second, |b| *a += *b).unwrap();
            let nested = insert("created in callback").unwrap();
            close::<&str>(&nested).unwrap();
        })
        .unwrap();
        with::<u32, _>(&first, |a| {
            assert_eq!(with::<u32, _>(&first, |b| *a + *b).unwrap(), 6);
            assert_eq!(close::<u32>(&first), Err(ResourceError::Borrowed));
        })
        .unwrap();
        close::<u32>(&first).unwrap();
        close::<u32>(&second).unwrap();
    }

    #[test]
    fn removal_transfers_ownership_and_destructors_can_use_the_registry() {
        struct Parent(String);
        impl Drop for Parent {
            fn drop(&mut self) {
                close::<u32>(&self.0).unwrap();
                let temporary = insert(7u32).unwrap();
                close::<u32>(&temporary).unwrap();
            }
        }
        let child = insert(5u32).unwrap();
        let parent = insert(Parent(child.clone())).unwrap();
        let value = remove::<Parent>(&parent).unwrap();
        assert_eq!(close::<Parent>(&parent), Err(ResourceError::NotFound));
        assert_eq!(with::<u32, _>(&child, |n| *n).unwrap(), 5);
        drop(value);
        assert_eq!(close::<u32>(&child), Err(ResourceError::NotFound));
    }

    #[test]
    fn native_registry_is_local_to_its_thread() {
        let first = insert(1u32).unwrap();
        let copied = first.clone();
        let second = std::thread::spawn(move || {
            let local = insert(2u32).unwrap();
            assert_eq!(
                with::<u32, _>(&copied, |n| *n),
                Err(ResourceError::NotFound)
            );
            close::<u32>(&local).unwrap();
            local
        })
        .join()
        .unwrap();
        assert_ne!(first, second);
        assert_eq!(
            with::<u32, _>(&second, |n| *n),
            Err(ResourceError::NotFound)
        );
        close::<u32>(&first).unwrap();
    }
}
