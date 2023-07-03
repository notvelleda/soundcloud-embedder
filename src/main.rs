#![feature(let_chains)]

pub mod api;

use hyper::{
    header::CONTENT_TYPE,
    service::{make_service_fn, service_fn},
    Body, Method, Request, Response, Server, StatusCode,
};
use log::error;
use std::{convert::Infallible, net::ToSocketAddrs};

pub const MAX_ARTIST_LEN: usize = 64;
pub const MAX_TITLE_LEN: usize = 64;
pub const MAX_DESCRIPTION_LEN: usize = 192;

fn make_embed_page(info: api::ResolveInfo) -> String {
    let permalink = html_escape::encode_quoted_attribute(info.permalink_url());
    let artwork_url = info.artwork_url().replace("-large.jpg", "-t500x500.jpg");
    let artwork_url = html_escape::encode_quoted_attribute(&artwork_url);
    let artist = html_escape::encode_quoted_attribute(info.artist_name());
    let title = html_escape::encode_quoted_attribute(info.title());
    let description = html_escape::encode_quoted_attribute(info.description());
    let ogp_kind = match info {
        api::ResolveInfo::Track(_) => "music.song",
        api::ResolveInfo::Playlist(_) => "music.playlist",
    };

    let embed_url = format!(
        "https://fxsoundcloud.com/embed?text={}&url={}",
        urlencoding::encode(&info.counts()),
        urlencoding::encode(info.permalink_url())
    );

    format!("<!DOCTYPE html>
<html lang=\"en\">
    <head>
        <link rel=\"canonical\" href=\"{permalink}\"/>
        <meta property=\"theme-color\" content=\"undefined\"/>
        <meta property=\"twitter:card\" content=\"player\"/>
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
</html>")
}

async fn uwu(request: Request<Body>) -> std::result::Result<Response<Body>, Infallible> {
    let mut response = Response::new(Body::empty());

    let token: api::ClientToken = toml::from_str(&std::fs::read_to_string("token.toml").unwrap()).unwrap();

    match (request.method(), request.uri().path()) {
        (&Method::GET, "/") => {
            *response.body_mut() = Body::from(":3c");
        }
        (&Method::GET, "/embed") => {
            // oembed endpoint
            let mut embed_text = "".to_string();
            let mut embed_url = "".to_string();

            for pair in request.uri().query().iter().flat_map(|q| q.split('&')) {
                let mut split = pair.split('=');

                match split.next() {
                    Some("text") => embed_text = urlencoding::decode(split.next().unwrap_or("")).unwrap_or_default().to_string(),
                    Some("url") => embed_url = urlencoding::decode(split.next().unwrap_or("")).unwrap_or_default().to_string(),
                    _ => (),
                }
            }

            let embed_text = html_escape::encode_quoted_attribute(&embed_text);
            let embed_url = html_escape::encode_quoted_attribute(&embed_url);

            response
                .headers_mut()
                .append(CONTENT_TYPE, "application/json".parse().unwrap());
            *response.body_mut() = Body::from(
                format!("{{\"author_name\":\"{embed_text}\",\"author_url\":\"{embed_url}\",\"provider_name\":\"soundcloud-embedder\",\"provider_url\":\"https://velleda.xyz/\",\"title\":\"SoundCloud\",\"type\":\"link\",\"version\":\"1.0\"}}"),
            );
        }
        (&Method::GET, path) => {
            // soundcloud url endpoint
            let absolute_uri = format!("https://soundcloud.com{path}");

            //*response.body_mut() = Body::from(format!("{:#?}", api_request(&make_resolve_url(&token.client_id, &new_uri), &token.auth).await));
            match api::resolve(&token, &absolute_uri).await {
                anyhow::Result::Ok(info) => {
                    response
                        .headers_mut()
                        .append(CONTENT_TYPE, "text/html".parse().unwrap());
                    *response.body_mut() = Body::from(make_embed_page(info));
                }
                Err(err) => {
                    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                    *response.body_mut() = Body::from(format!("error: {err:?}"));
                    error!("{err:?}");
                }
            }
        }
        _ => {
            *response.status_mut() = StatusCode::NOT_FOUND;
            *response.body_mut() = Body::from("404, silly!");
        }
    }

    std::result::Result::Ok(response) // lol, lmao
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let service =
        make_service_fn(|_| async { std::result::Result::Ok::<_, Infallible>(service_fn(uwu)) });

    let server =
        Server::bind(&"127.0.0.1:3621".to_socket_addrs().unwrap().next().unwrap()).serve(service);

    if let Err(err) = server.await {
        error!("{err:?}");
    }
}
