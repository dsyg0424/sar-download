use std::{collections::{HashMap, VecDeque}, sync::{Mutex, LazyLock}};
use tokio::sync::watch;

#[derive(Default)]
struct Registry { active: HashMap<String, watch::Sender<bool>>, early: VecDeque<String> }
static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(|| Mutex::new(Registry::default()));
pub struct Registration { id: String, pub receiver: watch::Receiver<bool> }
fn validate(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 80 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err("INVALID_SEARCH_ID".into());
    }
    Ok(())
}
pub fn register(id: &str) -> Result<Registration, String> {
    validate(id)?;
    let mut registry = REGISTRY.lock().map_err(|e| e.to_string())?;
    if registry.active.contains_key(id) { return Err("DUPLICATE_SEARCH_ID".into()); }
    let early = registry.early.iter().any(|value| value == id);
    registry.early.retain(|value| value != id);
    let (sender, receiver) = watch::channel(early);
    registry.active.insert(id.to_owned(), sender);
    Ok(Registration { id: id.to_owned(), receiver })
}
pub fn cancel(id: &str) -> Result<(), String> {
    validate(id)?;
    let mut registry = REGISTRY.lock().map_err(|e| e.to_string())?;
    if let Some(sender) = registry.active.get(id) { sender.send_replace(true); }
    else if !registry.early.iter().any(|value| value == id) {
        if registry.early.len() >= 64 { registry.early.pop_front(); }
        registry.early.push_back(id.to_owned());
    }
    Ok(())
}
impl Drop for Registration {
    fn drop(&mut self) { if let Ok(mut registry) = REGISTRY.lock() { registry.active.remove(&self.id); } }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_handles_early_delivery_and_targets_only_requested_query() {
        cancel("early-test").unwrap();
        let early = register("early-test").unwrap();
        assert!(*early.receiver.borrow());
        let one = register("active-one").unwrap();
        let two = register("active-two").unwrap();
        cancel("active-one").unwrap();
        assert!(*one.receiver.borrow());
        assert!(!*two.receiver.borrow());
        drop(one);
        assert!(!*register("active-one").unwrap().receiver.borrow());
    }
}
