use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use serde_json::Value;
use sqlitegraph::GraphEntity;

use super::{SessionSummary, WikiPage};

const BASE_TTL: Duration = Duration::from_secs(5);
const HOT_TTL: Duration = Duration::from_secs(30);
const HOT_HIT_THRESHOLD: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CacheDomain {
    Memory,
    Sessions,
    Events,
    Wiki,
    Knowledge,
    Search,
    Navigation,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum QueryCacheKey {
    QueryMemory {
        key: String,
        scope: Option<String>,
        project_id: Option<String>,
    },
    ListMemory {
        scope: Option<String>,
        project_id: Option<String>,
    },
    QuerySessions {
        project: Option<String>,
        last_n: i64,
        parent_id: Option<String>,
    },
    QueryEvents {
        session_id: Option<String>,
        event_type: Option<String>,
        limit: usize,
    },
    ListWikiPages {
        project_id: Option<String>,
    },
    QueryKnowledge {
        target: String,
        project_id: Option<String>,
    },
    LexicalSearch {
        query: String,
        k: usize,
        project_id: Option<String>,
        entity_kind: Option<String>,
    },
    Navigate {
        query: String,
        k: usize,
        depth: u32,
        project_id: Option<String>,
        entity_kind: Option<String>,
    },
    Hopgraph {
        query: String,
        k: usize,
        depth: u32,
        allowed_types_key: String,
        max_tokens: usize,
        project_id: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum QueryCacheValue {
    Entities(Vec<GraphEntity>),
    Sessions(Vec<SessionSummary>),
    Events(Vec<Value>),
    WikiPages(Vec<WikiPage>),
    Json(Value),
    SearchResults(Vec<super::SearchResult>),
    SubgraphViews(Vec<super::SubgraphView>),
}

struct CacheEntry {
    value: QueryCacheValue,
    generation: u64,
    expires_at: Instant,
    hits: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct RuntimeStats {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub memory_queries: u64,
    pub memory_writes: u64,
    pub session_queries: u64,
    pub session_writes: u64,
    pub event_queries: u64,
    pub event_writes: u64,
    pub knowledge_queries: u64,
    pub knowledge_writes: u64,
    pub wiki_queries: u64,
    pub wiki_writes: u64,
    pub search_queries: u64,
    pub navigation_queries: u64,
}

pub(crate) struct GraphRuntime {
    cache: RwLock<HashMap<QueryCacheKey, CacheEntry>>,
    memory_generation: AtomicU64,
    session_generation: AtomicU64,
    event_generation: AtomicU64,
    wiki_generation: AtomicU64,
    knowledge_generation: AtomicU64,
    search_generation: AtomicU64,
    navigation_generation: AtomicU64,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    memory_queries: AtomicU64,
    memory_writes: AtomicU64,
    session_queries: AtomicU64,
    session_writes: AtomicU64,
    event_queries: AtomicU64,
    event_writes: AtomicU64,
    knowledge_queries: AtomicU64,
    knowledge_writes: AtomicU64,
    wiki_queries: AtomicU64,
    wiki_writes: AtomicU64,
    search_queries: AtomicU64,
    navigation_queries: AtomicU64,
}

impl Default for GraphRuntime {
    fn default() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            memory_generation: AtomicU64::new(0),
            session_generation: AtomicU64::new(0),
            event_generation: AtomicU64::new(0),
            wiki_generation: AtomicU64::new(0),
            knowledge_generation: AtomicU64::new(0),
            search_generation: AtomicU64::new(0),
            navigation_generation: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            memory_queries: AtomicU64::new(0),
            memory_writes: AtomicU64::new(0),
            session_queries: AtomicU64::new(0),
            session_writes: AtomicU64::new(0),
            event_queries: AtomicU64::new(0),
            event_writes: AtomicU64::new(0),
            knowledge_queries: AtomicU64::new(0),
            knowledge_writes: AtomicU64::new(0),
            wiki_queries: AtomicU64::new(0),
            wiki_writes: AtomicU64::new(0),
            search_queries: AtomicU64::new(0),
            navigation_queries: AtomicU64::new(0),
        }
    }
}

impl GraphRuntime {
    pub(crate) fn cache_get(
        &self,
        key: &QueryCacheKey,
        domain: CacheDomain,
    ) -> Option<QueryCacheValue> {
        let generation = self.generation(domain);
        let now = Instant::now();
        let mut cache = self.cache.write();

        match cache.get_mut(key) {
            Some(entry) if entry.generation == generation && entry.expires_at > now => {
                entry.hits = entry.hits.saturating_add(1);
                entry.expires_at = now
                    + if entry.hits >= HOT_HIT_THRESHOLD {
                        HOT_TTL
                    } else {
                        BASE_TTL
                    };
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
                Some(entry.value.clone())
            }
            Some(_) => {
                cache.remove(key);
                self.cache_misses.fetch_add(1, Ordering::Relaxed);
                None
            }
            None => {
                self.cache_misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    pub(crate) fn cache_store(
        &self,
        key: QueryCacheKey,
        domain: CacheDomain,
        value: QueryCacheValue,
    ) {
        let generation = self.generation(domain);
        let entry = CacheEntry {
            value,
            generation,
            expires_at: Instant::now() + BASE_TTL,
            hits: 0,
        };
        let mut cache = self.cache.write();
        cache.insert(key, entry);
    }

    pub(crate) fn bump_generation(&self, domain: CacheDomain) {
        self.generation_cell(domain).fetch_add(1, Ordering::Relaxed);
        // Search and navigation caches depend on all entity domains.
        self.search_generation.fetch_add(1, Ordering::Relaxed);
        self.navigation_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn bump_navigation_generation(&self) {
        self.navigation_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_memory_query(&self) {
        self.memory_queries.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_memory_write(&self) {
        self.memory_writes.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_session_query(&self) {
        self.session_queries.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_session_write(&self) {
        self.session_writes.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_event_query(&self) {
        self.event_queries.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_event_write(&self) {
        self.event_writes.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_knowledge_query(&self) {
        self.knowledge_queries.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_knowledge_write(&self) {
        self.knowledge_writes.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_wiki_query(&self) {
        self.wiki_queries.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_wiki_write(&self) {
        self.wiki_writes.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_search_query(&self) {
        self.search_queries.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_navigation_query(&self) {
        self.navigation_queries.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> RuntimeStats {
        RuntimeStats {
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            memory_queries: self.memory_queries.load(Ordering::Relaxed),
            memory_writes: self.memory_writes.load(Ordering::Relaxed),
            session_queries: self.session_queries.load(Ordering::Relaxed),
            session_writes: self.session_writes.load(Ordering::Relaxed),
            event_queries: self.event_queries.load(Ordering::Relaxed),
            event_writes: self.event_writes.load(Ordering::Relaxed),
            knowledge_queries: self.knowledge_queries.load(Ordering::Relaxed),
            knowledge_writes: self.knowledge_writes.load(Ordering::Relaxed),
            wiki_queries: self.wiki_queries.load(Ordering::Relaxed),
            wiki_writes: self.wiki_writes.load(Ordering::Relaxed),
            search_queries: self.search_queries.load(Ordering::Relaxed),
            navigation_queries: self.navigation_queries.load(Ordering::Relaxed),
        }
    }

    fn generation(&self, domain: CacheDomain) -> u64 {
        self.generation_cell(domain).load(Ordering::Relaxed)
    }

    fn generation_cell(&self, domain: CacheDomain) -> &AtomicU64 {
        match domain {
            CacheDomain::Memory => &self.memory_generation,
            CacheDomain::Sessions => &self.session_generation,
            CacheDomain::Events => &self.event_generation,
            CacheDomain::Wiki => &self.wiki_generation,
            CacheDomain::Knowledge => &self.knowledge_generation,
            CacheDomain::Search => &self.search_generation,
            CacheDomain::Navigation => &self.navigation_generation,
        }
    }
}
