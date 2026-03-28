use std::hash::{Hash, Hasher};

use lru::LruCache;
use maxminddb::Reader;
use memmap2::Mmap;
use ntex::web;

const SHARD_SIZE: usize = 16;
const CACHE_SIZE: usize = 1024;

#[derive(serde::Deserialize)]
struct GeoIpQuery {
  ip: std::net::IpAddr,
}

#[derive(Clone, serde::Serialize)]
struct GeoIpResponse {
  country: Option<String>,
  city: Option<String>,
  asn: Option<String>,
}

struct CacheShard(std::sync::Mutex<LruCache<std::net::IpAddr, GeoIpResponse>>);

impl CacheShard {
  fn new() -> Self {
    let cache_size = std::num::NonZeroUsize::new(CACHE_SIZE).unwrap();
    Self(std::sync::Mutex::new(LruCache::new(cache_size)))
  }
  fn get(&self, ip: &std::net::IpAddr) -> Option<GeoIpResponse> {
    self.0.lock().ok()?.get(ip).cloned()
  }
  fn set(&self, ip: std::net::IpAddr, response: GeoIpResponse) {
    if let Ok(mut cache) = self.0.lock() {
      cache.put(ip, response);
    }
  }
}

struct AppInner(Reader<Mmap>, Reader<Mmap>, Vec<CacheShard>);

#[derive(Clone)]
struct AppState(std::sync::Arc<AppInner>);

impl AppState {
  fn open_mmap(path: &str) -> anyhow::Result<Reader<Mmap>> {
    Ok(Reader::from_source(unsafe {
      Mmap::map(&std::fs::File::open(path)?)?
    })?)
  }
  fn new() -> anyhow::Result<Self> {
    Ok(Self(std::sync::Arc::new(AppInner(
      Self::open_mmap("./mmdb/GeoLite2-City.mmdb")?,
      Self::open_mmap("./mmdb/GeoLite2-ASN.mmdb")?,
      (0..SHARD_SIZE).map(|_| CacheShard::new()).collect(),
    ))))
  }
  fn shard(&self, ip: &std::net::IpAddr) -> &CacheShard {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    ip.hash(&mut hasher);
    &self.0.2[(hasher.finish() as usize) % SHARD_SIZE]
  }
  fn lookup(&self, ip: &std::net::IpAddr) -> anyhow::Result<GeoIpResponse> {
    let shard = self.shard(ip);
    if let Some(v) = shard.get(ip) {
      return Ok(v.to_owned());
    }
    let (country, city) = self
      .0
      .0
      .lookup(*ip)?
      .decode::<maxminddb::geoip2::City>()?
      .map(|c| {
        (
          c.country.iso_code.map(|s| s.to_owned()),
          c.city.names.english.map(|s| s.to_owned()),
        )
      })
      .unwrap_or((None, None));
    let asn = self
      .0
      .1
      .lookup(*ip)?
      .decode::<maxminddb::geoip2::Asn>()?
      .and_then(|a| a.autonomous_system_organization.map(|s| s.to_owned()));
    let result = GeoIpResponse { country, city, asn };
    shard.set(*ip, result.to_owned());
    Ok(result)
  }
}

#[web::get("/geoip")]
pub async fn geoip_handler(
  app_state: web::types::State<AppState>,
  query: web::types::Query<GeoIpQuery>,
) -> web::HttpResponse {
  match app_state.lookup(&query.ip) {
    Err(_) => web::HttpResponse::InternalServerError().finish(),
    Ok(geoip) => web::HttpResponse::Ok().json(&geoip),
  }
}

#[ntex::main]
async fn main() -> anyhow::Result<()> {
  let app_state = AppState::new()?;
  let srv = web::HttpServer::new(async move || {
    web::App::new()
      .state(app_state.clone())
      .service(geoip_handler)
  })
  .workers(num_cpus::get());
  srv.bind("0.0.0.0:8585")?.run().await?;
  Ok(())
}
