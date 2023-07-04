#![feature(let_chains)]

pub mod api;

use anyhow::*;
use hyper::{
    header::{CONTENT_TYPE, HOST, LOCATION},
    service::{make_service_fn, service_fn},
    Body, Method, Request, Response, Server, StatusCode, server::conn::AddrIncoming,
};
use hyper_rustls::TlsAcceptor;
use lazy_static::lazy_static;
use log::{debug, error, info, warn};
use redis::{aio::ConnectionManager, AsyncCommands};
use regex::Regex;
use rustls::{Certificate, PrivateKey};
use serde::{Deserialize, Serialize};
use std::{
    convert::Infallible,
    fs::File,
    io::BufReader,
    net::ToSocketAddrs,
    path::{Path, PathBuf},
};

/// maximum length for artist names
pub const MAX_ARTIST_LEN: usize = 64;

/// maximum length for track titles
pub const MAX_TITLE_LEN: usize = 64;

/// maximum length for track description
pub const MAX_DESCRIPTION_LEN: usize = 192;

/// how long to cache song data for before making another api request, in seconds
pub const CACHE_TTL_SECS: usize = 28800; // 8 hours

/// the oembed provider url and the url to redirect the root page to
pub const WEBSITE_URL: &str = "https://github.com/notvelleda/soundcloud-embedder";

/// handle requests to the oembed endpoint
fn handle_oembed(request: Request<Body>) -> Result<Response<Body>> {
    let mut embed_text = "".to_string();
    let mut embed_url = "".to_string();

    for pair in request.uri().query().iter().flat_map(|q| q.split('&')) {
        let mut split = pair.split('=');

        match split.next() {
            Some("text") => embed_text = urlencoding::decode(split.next().unwrap_or_default())?.to_string(),
            Some("url") => embed_url = urlencoding::decode(split.next().unwrap_or_default())?.to_string(),
            _ => (),
        }
    }

    #[derive(Serialize)]
    struct OEmbed<'a> {
        version: &'a str,
        r#type: &'a str,
        title: &'a str,
        author_name: &'a str,
        author_url: &'a str,
        provider_name: &'a str,
        provider_url: &'a str,
    }

    let value = OEmbed {
        version: "1.0",
        r#type: "link",
        title: "SoundCloud",
        author_name: &embed_text,
        author_url: &embed_url,
        provider_name: "soundcloud-embedder",
        provider_url: WEBSITE_URL,
    };

    let mut response = Response::new(Body::from(serde_json::to_string(&value)?));
    response.headers_mut().append(CONTENT_TYPE, "application/json".parse()?);
    Result::Ok(response)
}

/// makes an html document containing embed information based on the given track info
fn make_embed_page(hostname: &str, info: api::ResolveInfo) -> String {
    let permalink = html_escape::encode_quoted_attribute(info.permalink_url());
    let artwork_url = info.artwork_url().replace("-large.jpg", "-t500x500.jpg"); // large isn't large enough
    let artwork_url = html_escape::encode_quoted_attribute(&artwork_url);
    let artist = html_escape::encode_quoted_attribute(info.artist_name());
    let title = html_escape::encode_quoted_attribute(info.title());
    let description = html_escape::encode_quoted_attribute(info.description());
    let ogp_kind = match info {
        api::ResolveInfo::Track(_) => "music.song",
        api::ResolveInfo::Playlist(_) => "music.playlist",
    };

    let embed_url = format!(
        "https://{}/oembed?text={}&url={}",
        hostname,
        urlencoding::encode(&info.counts()),
        urlencoding::encode(info.permalink_url())
    );

    format!(
        "<!DOCTYPE html>
<html lang=\"en\">
    <head>
        <link rel=\"canonical\" href=\"{permalink}\"/>
        <meta property=\"theme-color\" content=\"undefined\"/>
        <meta property=\"twitter:card\" content=\"summary\"/>
        <meta property=\"twitter:title\" content=\"{artist} - {title}\"/>
        <meta property=\"twitter:image\" content=\"{artwork_url}\"/>
        <meta property=\"twitter:description\" content=\"{description}\"/>
        <meta http-equiv=\"refresh\" content=\"0;url={permalink}\"/>
        <meta property=\"og:title\" content=\"{artist} - {title}\"/>
        <meta property=\"og:type\" content=\"{ogp_kind}\"/>
        <meta property=\"og:image\" content=\"{artwork_url}\"/>
        <meta property=\"og:image:width\" content=\"500\"/>
        <meta property=\"og:image:height\" content=\"500\"/>
        <meta property=\"og:url\" content=\"{permalink}\"/>
        <meta property=\"og:description\" content=\"{description}\"/>
        <meta property=\"og:site_name\" content=\"soundcloud-embedder\"/>
        <link rel=\"alternate\" href=\"{embed_url}\" type=\"application/json+oembed\" title=\"title\">
    </head>
    <body></body>
</html>
"
    )
}

lazy_static! {
    static ref VALID_URL: Regex = Regex::new("^/[^/]+/(?:sets/)?[^/]+(?:/(?:s-[^/]+)?)?$").unwrap();
}

/// handle requests to embed a soundcloud page
async fn handle_page(request: Request<Body>, mut conn: ConnectionManager) -> Result<Response<Body>> {
    let path = request.uri().path();
    let absolute_uri = format!("https://soundcloud.com{path}");

    if !VALID_URL.is_match(path) {
        // this url probably isn't valid, just redirect to soundcloud so there are no api requests for invalid data
        let mut response = Response::new(Body::from("invalid url, silly!"));
        *response.status_mut() = StatusCode::MOVED_PERMANENTLY;
        response.headers_mut().append(LOCATION, absolute_uri.parse()?);
        Ok(response)
    } else {
        let resolved = match conn.get::<&str, Option<String>>(path).await?.and_then(|s| serde_json::from_str(&s).ok()) {
            Some(resolved) => {
                debug!("cache hit for {path}");
                resolved
            }
            None => {
                // data isn't in cache, do an api request to get the info we need
                debug!("cache miss for {path}");

                let client_id = conn.get::<&str, String>("client_id").await.context("failed to get client id from database")?;
                let resolved = api::resolve(&client_id, &absolute_uri).await?;

                conn.set_ex::<&str, String, String>(path, serde_json::to_string(&resolved)?, CACHE_TTL_SECS).await?;

                resolved
            }
        };

        let hostname = request.headers().get(HOST).and_then(|v| v.to_str().ok()).unwrap_or("unknown-host");
        let mut response = Response::new(Body::from(make_embed_page(hostname, resolved)));
        response.headers_mut().append(CONTENT_TYPE, "text/html".parse()?);
        Ok(response)
    }
}

/// checks what kind of request was received and handles it accordingly
async fn handle_request(request: Request<Body>, conn: ConnectionManager) -> Result<Response<Body>> {
    match (request.method(), request.uri().path()) {
        (&Method::GET, "/") => {
            let mut response = Response::new(Body::from(":3c"));
            *response.status_mut() = StatusCode::MOVED_PERMANENTLY;
            response.headers_mut().append(LOCATION, WEBSITE_URL.parse()?);
            Ok(response)
        }
        (&Method::GET, "/oembed") => handle_oembed(request),
        (&Method::GET, _) => handle_page(request, conn).await,
        _ => {
            let mut response = Response::new(Body::from("404, silly!"));
            *response.status_mut() = StatusCode::NOT_FOUND;
            Ok(response)
        }
    }
}

/// wrapper over handle_request() to properly handle errors
async fn handle_request_wrapper(request: Request<Body>, conn: ConnectionManager) -> Result<Response<Body>, Infallible> {
    match handle_request(request, conn).await {
        Result::Ok(response) => Result::Ok(response),
        Err(err) => {
            error!("error in handle_request: {err:?}");

            let mut response = Response::new(Body::from(format!("something bad happened! {err}\n")));
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            Result::Ok(response)
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Config {
    redis_address: String,
    listen_address: String,
    client_id: String,
    certs_path: PathBuf,
    private_key_path: PathBuf,
}

// ssl support adapted from https://github.com/rustls/hyper-rustls/blob/main/examples/server.rs

/// loads public certificates from the file at the given path
fn load_certs(path: &Path) -> Result<Vec<Certificate>> {
    // Open certificate file.
    let certfile = File::open(path)?;
    let mut reader = BufReader::new(certfile);

    // Load and return certificate.
    let certs = rustls_pemfile::certs(&mut reader)?;
    Ok(certs.into_iter().map(Certificate).collect())
}

/// loads a private key from the file at the given path
fn load_private_key(filename: &Path) -> Result<PrivateKey> {
    // Open keyfile.
    let keyfile = File::open(filename)?;
    let mut reader = BufReader::new(keyfile);

    // Load and return a single private key.
    let keys = rustls_pemfile::rsa_private_keys(&mut reader)?;
    if keys.len() != 1 {
        return Err(anyhow!("expected a single private key"));
    }

    Ok(rustls::PrivateKey(keys[0].clone()))
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let config_path = Path::new("config.toml");

    if !config_path.exists() {
        error!("config file {config_path:?} doesn't exist");

        match std::fs::write(config_path, toml::to_string_pretty(&Config::default()).unwrap()) {
            Result::Ok(_) => error!("created a blank config file, please populate it with options"),
            Err(err) => error!("failed to create a blank config file: {err}"),
        }

        return;
    }

    // read config from file
    let config_text = match std::fs::read_to_string(config_path) {
        Result::Ok(text) => text,
        Err(err) => {
            error!("failed to read config: {err}");
            return;
        }
    };
    let config: Config = match toml::from_str(&config_text) {
        Result::Ok(config) => config,
        Err(err) => {
            error!("failed to parse config: {err}");
            return;
        }
    };

    // load certs and privkey from disk
    let certs = match load_certs(&config.certs_path) {
        Result::Ok(certs) => Some(certs),
        Err(err) => {
            error!("failed to load certs: {err}");
            None
        }
    };
    let privkey = match load_private_key(&config.private_key_path) {
        Result::Ok(certs) => Some(certs),
        Err(err) => {
            error!("failed to load private key: {err}");
            None
        }
    };

    let client = redis::Client::open(config.redis_address).unwrap();
    let mut con_manager = ConnectionManager::new(client).await.unwrap();

    con_manager.set::<&str, String, String>("client_id", config.client_id).await.unwrap();

    let addr = config.listen_address.to_socket_addrs().unwrap().next().unwrap();
    info!("server listening on {addr:?}");

    if let Some(certs) = certs && let Some(privkey) = privkey {
        let incoming = AddrIncoming::bind(&addr).unwrap();
        let acceptor = TlsAcceptor::builder()
            .with_single_cert(certs, privkey).unwrap()
            .with_all_versions_alpn()
            .with_incoming(incoming);

        // such an awful api pattern istg
        let service = make_service_fn(move |_| {
            let conn = con_manager.clone();
            async move { std::result::Result::Ok::<_, Infallible>(service_fn(move |req| handle_request_wrapper(req, conn.clone()))) }
        });

        if let Err(err) = Server::builder(acceptor).serve(service).await {
            error!("{err}");
        }
    } else {
        warn!("couldn't load certs or privkey, defaulting to insecure http");

        // has to be duplicated because the ignored closure argument can differ
        let service = make_service_fn(move |_| {
            let conn = con_manager.clone();
            async move { std::result::Result::Ok::<_, Infallible>(service_fn(move |req| handle_request_wrapper(req, conn.clone()))) }
        });

        if let Err(err) = Server::bind(&addr).serve(service).await {
            error!("{err}");
        }
    }
}
