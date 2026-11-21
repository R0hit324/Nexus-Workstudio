//! Plugin events: versioned envelopes over the Part 4 bus.
//!
//! Plugin-generated events must follow a versioned schema, pass payload
//! size validation, honor permission gating, and rate limits — arbitrary
//! unbounded event injection is not allowed (spec sections 29–30, 89).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::context::PluginContext;
use crate::error::{PluginError, PluginResult};
use crate::id::PluginId;
use crate::permission::PluginPermission;

/// Maximum serialized payload of one plugin event.
pub const MAX_PLUGIN_EVENT_PAYLOAD: usize = 64 * 1024;

/// Default publish budget sustained per plugin.
pub const DEFAULT_EVENTS_PER_MINUTE: u64 = 240;

/// Schema version of the envelope itself (independent of plugin payloads).
pub const ENVELOPE_SCHEMA_VERSION: u32 = 1;

/// A plugin-generated event envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginEventEnvelope {
    /// Originating plugin.
    pub plugin_id: PluginId,
    /// Dotted event type, namespaced by plugin leaf, e.g. `git.branch-switched`.
    pub event_type: String,
    /// Plugin-owned payload schema version.
    pub schema_version: u32,
    /// Opaque payload (plugin-defined format; usually JSON).
    pub payload: Vec<u8>,
}

impl PluginEventEnvelope {
    /// Validates event_type + payload size without touching contents.
    ///
    /// # Errors
    /// [`PluginError::internal`] when malformed.
    pub fn validate(&self) -> PluginResult<()> {
        if self.event_type.is_empty()
            || !self
                .event_type
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
        {
            return Err(PluginError::internal(format!(
                "invalid plugin event type `{}`",
                self.event_type
            )));
        }
        if self.schema_version == 0 {
            return Err(PluginError::internal(
                "plugin event schema_version must be > 0",
            ));
        }
        if self.payload.len() > MAX_PLUGIN_EVENT_PAYLOAD {
            return Err(PluginError::internal(format!(
                "plugin event payload {} exceeds limit {} bytes",
                self.payload.len(),
                MAX_PLUGIN_EVENT_PAYLOAD
            )));
        }
        Ok(())
    }
}

/// Broad routing categories the hub uses for subscriptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluginEventCategory {
    Workspace,
    Build,
    Terminal,
    Cube,
    Plugin,
}

/// A registered subscription's selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginSubscription {
    /// Which categories the subscriber wants.
    pub categories: Vec<PluginEventCategory>,
    /// Optional event-type prefix filter.
    pub event_type_prefix: Option<String>,
    /// Restrict to one cube (empty scopes subscribe to all).
    pub cube_scopes: Vec<PluginScopeKey>,
}

/// Scope key for subscriptions; mirrors [`crate::context::PluginScope`]
/// but hashable and comparable without the enum's internal types.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PluginScopeKey {
    Server,
    Client,
    Cube(nexus_core::CubeId),
}

/// A live subscription; dropping it unsubscribes (deterministic cleanup).
pub struct PluginSubscriptionHandle {
    id: u64,
    hub_subscribers: Arc<HubSubscribers>,
}

impl PluginSubscriptionHandle {
    /// Pulls all currently buffered matching events (non-blocking).
    #[must_use]
    pub fn drain(&self) -> Vec<PluginEventEnvelope> {
        self.hub_subscribers.drain(self.id)
    }
}

impl Drop for PluginSubscriptionHandle {
    fn drop(&mut self) {
        self.hub_subscribers.unsubscribe(self.id);
    }
}

#[derive(Default)]
struct HubSubscribers {
    next_id: AtomicU64,
    entries: RwLock<HashMap<u64, SubscriberEntry>>,
}

#[derive(Debug, Clone)]
struct SubscriberEntry {
    subscription: PluginSubscription,
    buffer: std::collections::VecDeque<PluginEventEnvelope>,
    capacity: usize,
    #[allow(dead_code)]
    owner: PluginId,
}

impl HubSubscribers {
    fn unsubscribe(&self, id: u64) {
        self.entries.write().expect("subscriber lock").remove(&id);
    }

    fn drain(&self, id: u64) -> Vec<PluginEventEnvelope> {
        let mut entries = self.entries.write().expect("subscriber lock");
        entries
            .get_mut(&id)
            .map(|entry| entry.buffer.drain(..).collect())
            .unwrap_or_default()
    }

    fn fanout(&self, envelope: &PluginEventEnvelope, category: PluginEventCategory) -> usize {
        let mut entries = self.entries.write().expect("subscriber lock");
        let mut delivered = 0;
        for entry in entries.values_mut() {
            let matches = entry.subscription.categories.contains(&category)
                && entry
                    .subscription
                    .event_type_prefix
                    .as_ref()
                    .map_or(true, |prefix| envelope.event_type.starts_with(prefix));
            if matches {
                while entry.buffer.len() >= entry.capacity {
                    entry.buffer.pop_front();
                }
                entry.buffer.push_back(envelope.clone());
                delivered += 1;
            }
        }
        delivered
    }
}

/// The facade plugins get through [`crate::context::PluginServices`].
#[async_trait]
pub trait PluginEventApi: Send + Sync {
    /// Publishes a plugin event after validation, permission check, and rate
    /// limiting.
    ///
    /// # Errors
    /// Validation or rate-limit failures.
    async fn publish(
        &self,
        publisher: &PluginContext,
        envelope: PluginEventEnvelope,
        category: PluginEventCategory,
    ) -> PluginResult<usize>;

    /// Subscribes; the returned handle unsubscribes on drop.
    async fn subscribe(
        &self,
        subscriber: &PluginContext,
        subscription: PluginSubscription,
    ) -> PluginResult<PluginSubscriptionHandle>;
}

#[derive(Clone)]
struct RateWindow {
    window_started: Instant,
    seen: u64,
}

/// Event hub (crate-local implementation).
pub struct PluginEventHub {
    events_publish_enabled: bool,
    events_subscribe_enabled: bool,
    per_minute: u64,
    subscribers: Arc<HubSubscribers>,
    rates: RwLock<HashMap<PluginId, RateWindow>>,
    dropped: Arc<AtomicU64>,
}

impl PluginEventHub {
    /// Builds a hub from granted permissions and a configured rate limit.
    #[must_use]
    pub fn new(
        events_publish_enabled: bool,
        events_subscribe_enabled: bool,
        per_minute: u64,
    ) -> Self {
        Self {
            events_publish_enabled,
            events_subscribe_enabled,
            per_minute: if per_minute == 0 {
                DEFAULT_EVENTS_PER_MINUTE
            } else {
                per_minute
            },
            subscribers: Arc::new(HubSubscribers::default()),
            rates: RwLock::new(HashMap::new()),
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Number of events dropped for rate limiting, for observability.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn check_rate(&self, publisher: &PluginContext) -> PluginResult<()> {
        let now = Instant::now();
        let mut rates = self.rates.write().expect("rate lock");
        let window = rates
            .entry(publisher.plugin_id().clone())
            .or_insert(RateWindow {
                window_started: now,
                seen: 0,
            });
        if now.duration_since(window.window_started) >= Duration::from_secs(60) {
            window.window_started = now;
            window.seen = 0;
        }
        if window.seen >= self.per_minute {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return Err(PluginError::new(
                Some(publisher.plugin_id().clone()),
                "events.rate-limit",
                format!("event budget of {} per minute exceeded", self.per_minute),
            ));
        }
        window.seen += 1;
        Ok(())
    }
}

#[async_trait]
impl PluginEventApi for PluginEventHub {
    async fn publish(
        &self,
        publisher: &PluginContext,
        envelope: PluginEventEnvelope,
        category: PluginEventCategory,
    ) -> PluginResult<usize> {
        if !self.events_publish_enabled || !publisher.allows(PluginPermission::EventsPublish) {
            return Err(PluginError::new(
                Some(publisher.plugin_id().clone()),
                "events.publish",
                "plugin lacks `events.publish` permission",
            ));
        }
        let envelope = {
            let mut envelope = envelope;
            envelope.plugin_id = publisher.plugin_id().clone();
            envelope
        };
        envelope.validate().map_err(|e| {
            PluginError::new(
                Some(publisher.plugin_id().clone()),
                "events.validate",
                e.message,
            )
        })?;
        self.check_rate(publisher)?;
        let delivered = self.subscribers.fanout(&envelope, category);
        Ok(delivered)
    }

    async fn subscribe(
        &self,
        subscriber: &PluginContext,
        subscription: PluginSubscription,
    ) -> PluginResult<PluginSubscriptionHandle> {
        if !self.events_subscribe_enabled || !subscriber.allows(PluginPermission::EventsSubscribe) {
            return Err(PluginError::new(
                Some(subscriber.plugin_id().clone()),
                "events.subscribe",
                "plugin lacks `events.subscribe` permission",
            ));
        }
        if subscription.categories.is_empty() {
            return Err(PluginError::new(
                Some(subscriber.plugin_id().clone()),
                "events.subscribe",
                "subscription must include at least one category",
            ));
        }
        let hub_subscribers = Arc::clone(&self.subscribers);
        let id = hub_subscribers.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let entry = SubscriberEntry {
            subscription: subscription.clone(),
            buffer: std::collections::VecDeque::new(),
            capacity: 1024,
            owner: subscriber.plugin_id().clone(),
        };
        hub_subscribers
            .entries
            .write()
            .expect("subscriber lock")
            .insert(id, entry);
        Ok(PluginSubscriptionHandle {
            id,
            hub_subscribers,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::PluginScope;
    use crate::error::PluginResult;
    use crate::permission::GrantedPermissions;

    fn plugin_id() -> PluginId {
        PluginId::new("org.nexus.tester").unwrap()
    }

    fn pub_context() -> PluginContext {
        PluginContext::new(
            plugin_id(),
            PluginScope::Server,
            GrantedPermissions::from_set([PluginPermission::EventsPublish]),
        )
    }

    fn sub_context() -> PluginContext {
        PluginContext::new(
            plugin_id(),
            PluginScope::Server,
            GrantedPermissions::from_set([PluginPermission::EventsSubscribe]),
        )
    }

    fn envelope(event_type: &str) -> PluginEventEnvelope {
        PluginEventEnvelope {
            plugin_id: plugin_id(),
            event_type: String::from(event_type),
            schema_version: 1,
            payload: b"{}".to_vec(),
        }
    }

    fn hub() -> PluginEventHub {
        PluginEventHub::new(true, true, 240)
    }

    #[tokio::test]
    async fn publish_requires_permission() {
        let hub = hub();
        let denied = PluginContext::new(
            plugin_id(),
            PluginScope::Server,
            GrantedPermissions::default(),
        );
        let err = hub
            .publish(
                &denied,
                envelope("tester.ping"),
                PluginEventCategory::Plugin,
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("events.publish"));
    }

    #[tokio::test]
    async fn publish_then_subscriber_sees() {
        let hub = hub();
        let handle = hub
            .subscribe(
                &sub_context(),
                PluginSubscription {
                    categories: vec![PluginEventCategory::Plugin],
                    event_type_prefix: None,
                    cube_scopes: vec![],
                },
            )
            .await
            .unwrap();
        let delivered = hub
            .publish(
                &pub_context(),
                envelope("tester.ping"),
                PluginEventCategory::Plugin,
            )
            .await
            .unwrap();
        assert_eq!(delivered, 1);
        assert_eq!(handle.drain().len(), 1);
    }

    #[tokio::test]
    async fn filter_by_prefix() {
        let hub = hub();
        let handle = hub
            .subscribe(
                &sub_context(),
                PluginSubscription {
                    categories: vec![PluginEventCategory::Plugin],
                    event_type_prefix: Some(String::from("git.")),
                    cube_scopes: vec![],
                },
            )
            .await
            .unwrap();
        let seen = hub
            .publish(
                &pub_context(),
                envelope("tester.ping"),
                PluginEventCategory::Plugin,
            )
            .await
            .unwrap();
        assert_eq!(seen, 0, "prefix filter excludes unrelated events");
        let seen = hub
            .publish(
                &pub_context(),
                envelope("git.branch"),
                PluginEventCategory::Plugin,
            )
            .await
            .unwrap();
        assert_eq!(seen, 1);
        drop(handle.drain());
    }

    #[tokio::test]
    async fn drop_handle_unsubscribes() {
        let hub = hub();
        let handle = hub
            .subscribe(
                &sub_context(),
                PluginSubscription {
                    categories: vec![PluginEventCategory::Plugin],
                    event_type_prefix: None,
                    cube_scopes: vec![],
                },
            )
            .await
            .unwrap();
        drop(handle);
        let delivered = hub
            .publish(
                &pub_context(),
                envelope("tester.ping"),
                PluginEventCategory::Plugin,
            )
            .await
            .unwrap();
        assert_eq!(delivered, 0);
    }

    #[tokio::test]
    async fn oversize_payload_rejected() {
        let hub = hub();
        let mut big = envelope("tester.big");
        big.payload = vec![0u8; MAX_PLUGIN_EVENT_PAYLOAD + 1];
        let err = hub
            .publish(&pub_context(), big, PluginEventCategory::Plugin)
            .await
            .unwrap_err();
        assert!(err.message.contains("exceeds limit"));
    }

    #[tokio::test]
    async fn rate_limited() {
        let hub = PluginEventHub::new(true, true, 2);
        hub.publish(
            &pub_context(),
            envelope("tester.a"),
            PluginEventCategory::Plugin,
        )
        .await
        .unwrap();
        hub.publish(
            &pub_context(),
            envelope("tester.b"),
            PluginEventCategory::Plugin,
        )
        .await
        .unwrap();
        let err = hub
            .publish(
                &pub_context(),
                envelope("tester.c"),
                PluginEventCategory::Plugin,
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("budget"), "third publish hits the cap");
        assert!(hub.dropped() >= 1);
    }

    #[tokio::test]
    async fn invalid_event_type_rejected() {
        let hub = hub();
        let err = hub
            .publish(
                &pub_context(),
                envelope("Invalid Type!"),
                PluginEventCategory::Plugin,
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("invalid plugin event type"));
    }

    #[tokio::test]
    async fn subscribe_requires_category() {
        let hub = hub();
        let err: PluginResult<PluginSubscriptionHandle> = hub
            .subscribe(
                &sub_context(),
                PluginSubscription {
                    categories: vec![],
                    event_type_prefix: None,
                    cube_scopes: vec![],
                },
            )
            .await;
        assert!(err.is_err());
    }
}
