use anyhow::Result;
use std::sync::{Arc, Mutex, MutexGuard};

pub(crate) trait LockExt<T> {
    fn lock_ctx(&self, ctx: &str) -> Result<MutexGuard<'_, T>>;
}

impl<T> LockExt<T> for Arc<Mutex<T>> {
    fn lock_ctx(&self, ctx: &str) -> Result<MutexGuard<'_, T>> {
        self.lock()
            .map_err(|e| anyhow::anyhow!("db lock poisoned in {ctx}: {e}"))
    }
}

impl<T> LockExt<T> for Mutex<T> {
    fn lock_ctx(&self, ctx: &str) -> Result<MutexGuard<'_, T>> {
        self.lock()
            .map_err(|e| anyhow::anyhow!("mutex poisoned in {ctx}: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn arc_mutex_lock_ctx_works() {
        let m = Arc::new(Mutex::new(42u32));
        let guard = m.lock_ctx("test-arc").unwrap();
        assert_eq!(*guard, 42);
    }

    #[test]
    fn mutex_lock_ctx_works() {
        let m = Mutex::new("hello");
        let guard = m.lock_ctx("test-plain").unwrap();
        assert_eq!(*guard, "hello");
    }
}
