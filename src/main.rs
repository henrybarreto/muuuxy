use std::{env, io::Error, net::IpAddr, str::FromStr, sync::Arc, time::Duration};

use tokio::net::{self, TcpListener};

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

use serde::Deserialize;

use tracing::{debug, error, info, level_filters::LevelFilter, trace, warn};
use tracing_subscriber::EnvFilter;

use http::{Method, Proxy, StatusCode, Url, header, redirect::Policy};

use axum::{
    Router,
    body::Body,
    extract::{Extension, Query},
    http::{HeaderName, HeaderValue, Uri, uri::Scheme},
    response::{IntoResponse, Response},
    routing::get,
    serve,
};

use tower::ServiceBuilder;
use tower_http::{
    self,
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    propagate_header::PropagateHeaderLayer,
    trace::TraceLayer,
};

use rand::{Rng, distr::Alphanumeric, rng};

fn generate_key(len: usize) -> String {
    rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect::<String>()
}

const DEFAULT_MUUUXY_SERVER_SCHEME: &str = "http";
const DEFAULT_MUUUXY_SERVER_HOST: &str = "0.0.0.0";
const DEFAULT_MUUUXY_SERVER_PORT: &str = "3000";
const DEFAULT_MUUUXY_SERVER_DOMAIN: &str = "localhost:3000";

pub struct State {
    scheme: String,
    host: String,
    port: String,
    domain: String,
    proxy: Option<String>,
    key: String,
}

impl State {
    pub fn new(
        scheme: String,
        host: String,
        port: String,
        domain: String,
        proxy: Option<String>,
        key: String,
    ) -> Self {
        Self {
            scheme,
            host,
            port,
            domain,
            proxy,
            key,
        }
    }
}

async fn healthz() -> impl IntoResponse {
    let response = Response::builder()
        .status(StatusCode::OK)
        .body(Body::empty())
        .unwrap();

    return response;
}

#[derive(Deserialize)]
struct ProxyParams {
    url: String,
    key: String,
}

async fn proxy(params: Query<ProxyParams>, state: Extension<Arc<State>>) -> impl IntoResponse {
    let params: ProxyParams = params.0;

    let response_builder = Response::builder();

    if params.key != state.key {
        return response_builder
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from("key is invalid"))
            .unwrap();
    }

    let url_to_proxy = params.url;
    if url_to_proxy.is_empty() {
        return response_builder
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from("path cannot be empty"))
            .unwrap();
    }

    let key = params.key;
    if key.is_empty() {
        return response_builder
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from("key cannot be empty"))
            .unwrap();
    }

    info!(path = url_to_proxy, key = key, "proxy route called");

    let uri = Uri::from_str(&url_to_proxy).unwrap();

    let host = uri.host().unwrap();
    debug!(host = host, "url host");

    let scheme = uri.scheme();
    let port: u16 = match scheme {
        Some(s) if *s == Scheme::HTTP => 80,
        Some(s) if *s == Scheme::HTTPS => 443,
        Some(_) => {
            return response_builder
                .status(StatusCode::BAD_REQUEST)
                .body(Body::empty())
                .unwrap();
        }
        None => {
            return response_builder
                .status(StatusCode::BAD_REQUEST)
                .body(Body::empty())
                .unwrap();
        }
    };

    debug!(port = port, "url port");

    let addrs = net::lookup_host(format!("{}:{}", host, port))
        .await
        .unwrap();

    for addr in addrs {
        let ip = addr.ip();

        match ip {
            IpAddr::V4(v4) => {
                debug!(ip = v4.to_string(), "IPv4 address resolved");

                // NOTE: Trys to avoid Server-Side Request Forgery (SSRF).
                if v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_multicast()
                    || v4.is_broadcast()
                    || v4.is_unspecified()
                {
                    return response_builder
                        .status(StatusCode::BAD_REQUEST)
                        .header(
                            header::CONTENT_TYPE,
                            HeaderValue::from_static("application/octet-stream"),
                        )
                        .header(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"))
                        .body(Body::empty())
                        .unwrap();
                }
            }
            IpAddr::V6(v6) => {
                debug!(ip = v6.to_string(), "IPv6 address resolved");

                // NOTE: Trys to avoid Server-Side Request Forgery (SSRF).
                if v6.is_loopback() || v6.is_multicast() || v6.is_unspecified() {
                    return response_builder
                        .status(StatusCode::BAD_REQUEST)
                        .header(
                            header::CONTENT_TYPE,
                            HeaderValue::from_static("application/octet-stream"),
                        )
                        .header(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"))
                        .body(Body::empty())
                        .unwrap();
                }
            }
        };
    }

    const HTTP_BODY_MAX_LENGTH: usize = 50 * 1_000_000;
    const HTTP_CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
    const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
    const HTTP_USER_AGENT: &str = "muuuxy/1.0";
    const HTTP_MAX_REDIRECTS: usize = 1;

    // TODO: Check if the file ends with `.m3u8`.
    // TODO: Find better timeout value.
    let mut builder = http::ClientBuilder::new()
        // NOTE: Trys to avoid slowloris / connection flooding.
        .connect_timeout(HTTP_CONNECTION_TIMEOUT)
        .timeout(HTTP_TIMEOUT)
        // NOTE: Trys to avoid bait-and-switch.
        .redirect(Policy::limited(HTTP_MAX_REDIRECTS))
        .referer(false)
        .https_only(true)
        .user_agent(HTTP_USER_AGENT);

    builder = if let Some(proxy) = &state.proxy {
        debug!(proxy = proxy, "using proxy on proxied url");

        builder.proxy(Proxy::all(proxy).unwrap())
    } else {
        builder
    };

    let client = builder.build().unwrap();

    let response = match client.get(&url_to_proxy).send().await {
        Ok(r) => r,
        Err(e) => {
            error!("failed to perform request on the proxied url: {}", e);

            return response_builder
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("failed to perform request on the proxied url"))
                .unwrap();
        }
    };

    if response.status() != StatusCode::OK {
        error!(
            url = url_to_proxy,
            status = response.status().to_string(),
            "response from proxied server returned with a non 200 code"
        );

        return response_builder
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from(
                "request to proxied server returned a non 200 status",
            ))
            .unwrap();
    }

    let headers = response.headers().clone();

    let body = response.bytes().await.unwrap();
    if body.len() > HTTP_BODY_MAX_LENGTH {
        info!(
            length = body.len(),
            max = HTTP_BODY_MAX_LENGTH,
            "content length of proxied request is great than max allowed"
        );

        return response_builder
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from(
                "content length of proxied request is great than max allowed",
            ))
            .unwrap();
    }

    let playlist = match m3u8::parse_playlist(&body) {
        Ok((_, playlist)) => playlist,
        _ => {
            // NOTE: When the data isn't a playlist, we are considering it a binary chunk. We have
            // to check if it is right.
            let len = body.len();
            let len_as_string = len.to_string();

            return response_builder
                .status(StatusCode::OK)
                .header(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/octet-stream"),
                )
                .header(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"))
                .header(
                    header::CONTENT_LENGTH,
                    HeaderValue::from_str(&len_as_string).unwrap(),
                )
                .body(Body::from(body))
                .unwrap();
        }
    };

    match playlist {
        m3u8::Playlist::MasterPlaylist(mut master) => {
            trace!("master playlist got");

            master.variants = master
                .variants
                .into_iter()
                .map(|mut item| {
                    let item_uri = item.uri;
                    debug!(item_uri = item_uri, "master item url");

                    let uri: String = match Url::parse(&item_uri) {
                        Ok(u) => u.to_string(),
                        Err(_) => {
                            // NOTE: If the URL received as item is relative, we should override it
                            // to match the targetting server.
                            let mut u = Url::parse(&url_to_proxy).unwrap();

                            let mut segments = u.path_segments_mut().unwrap();
                            segments.pop();
                            // NOTE: Drop mutable reference to u;
                            drop(segments);

                            format!("{}/{}", u, item_uri)
                        }
                    };

                    let encoded_uri =
                        utf8_percent_encode(&format!("{}", uri), NON_ALPHANUMERIC).to_string();

                    item.uri = format!(
                        "{}://{}/proxy?key={}&url={}",
                        state.scheme, state.domain, key, encoded_uri
                    );

                    item
                })
                .collect();

            let mut master_buffer: Vec<u8> = Vec::new();
            master.write_to(&mut master_buffer).unwrap();

            let len = master_buffer.len();
            let len_as_string = len.to_string();

            let content_type = if let Some(conent_type) = headers.get("content-type") {
                conent_type
            } else {
                error!("proxied url doesn't have the required content-type header");

                return response_builder
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from(
                        "proxied url doesn't have the required content-type header",
                    ))
                    .unwrap();
            };

            return response_builder
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(
                    header::CONTENT_LENGTH,
                    HeaderValue::from_str(&len_as_string).unwrap(),
                )
                .body(Body::from(master_buffer))
                .unwrap();
        }
        m3u8::Playlist::MediaPlaylist(mut media) => {
            trace!("media playlist got");

            media.segments = media
                .segments
                .into_iter()
                .map(|mut item| {
                    let item_uri = item.uri;
                    debug!(item_uri = item_uri, "media item url");

                    let uri: String = match Url::parse(&item_uri) {
                        Ok(u) => u.to_string(),
                        Err(_) => {
                            // NOTE: If the URL received as item is relative, we should override it
                            // to match the targetting server.
                            let mut u = Url::parse(&url_to_proxy).unwrap();

                            let mut segments = u.path_segments_mut().unwrap();
                            segments.pop();
                            // NOTE: Drop mutable reference to u;
                            drop(segments);

                            format!("{}/{}", u, item_uri)
                        }
                    };

                    let encoded_uri =
                        utf8_percent_encode(&format!("{}", uri), NON_ALPHANUMERIC).to_string();

                    item.uri = format!(
                        "{}://{}/proxy?key={}&url={}",
                        state.scheme, state.domain, key, encoded_uri
                    );

                    item
                })
                .collect();

            let mut media_buffer: Vec<u8> = Vec::new();
            media.write_to(&mut media_buffer).unwrap();

            let len = media_buffer.len();
            let len_as_string = len.to_string();

            let content_type = if let Some(content_type) = headers.get("content-type") {
                content_type
            } else {
                error!("proxied url doesn't have the required content-type header");

                return response_builder
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from(
                        "proxied url doesn't have the required content-type header",
                    ))
                    .unwrap();
            };

            return response_builder
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(
                    header::CONTENT_LENGTH,
                    HeaderValue::from_str(&len_as_string).unwrap(),
                )
                .body(Body::from(media_buffer))
                .unwrap();
        }
    };
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_env_var("LOG")
                .with_default_directive(LevelFilter::INFO.into())
                .from_env()
                .unwrap(),
        )
        .init();

    info!("muuuxy server starting");

    let server_scheme = match env::var("MUUUXY_SERVER_SCHEME") {
        Ok(addr) => addr,
        Err(_) => {
            warn!(
                value = DEFAULT_MUUUXY_SERVER_SCHEME,
                "MUUUXY_SERVER_SCHEME not set, using default",
            );

            DEFAULT_MUUUXY_SERVER_SCHEME.to_string()
        }
    };

    let server_host = match env::var("MUUUXY_SERVER_HOST") {
        Ok(addr) => addr,
        Err(_) => {
            warn!(
                value = DEFAULT_MUUUXY_SERVER_HOST,
                "MUUUXY_SERVER_HOST not set, using default",
            );

            DEFAULT_MUUUXY_SERVER_HOST.to_string()
        }
    };

    let server_port = match env::var("MUUUXY_SERVER_PORT") {
        Ok(addr) => addr,
        Err(_) => {
            warn!(
                value = DEFAULT_MUUUXY_SERVER_PORT,
                "MUUUXY_SERVER_PORT not set, using default",
            );

            DEFAULT_MUUUXY_SERVER_PORT.to_string()
        }
    };

    let server_domain = match env::var("MUUUXY_SERVER_DOMAIN") {
        Ok(addr) => addr,
        Err(_) => {
            warn!(
                value = DEFAULT_MUUUXY_SERVER_DOMAIN,
                "MUUUXY_SERVER_DOMAIN not set, using default",
            );

            DEFAULT_MUUUXY_SERVER_DOMAIN.to_string()
        }
    };

    let server_proxy = match env::var("MUUUXY_SERVER_PROXY") {
        Ok(addr) => {
            info!(proxy = addr, "using proxy on proxied requests");

            Some(addr)
        }
        Err(_) => {
            warn!("MUUUXY_SERVER_PROXY not set, using default",);

            None
        }
    };

    const GENERATE_KEY_LENGTH: usize = 32;

    let server_key = match env::var("MUUUXY_SERVER_KEY") {
        Ok(key) => key,
        Err(_) => {
            let key = generate_key(GENERATE_KEY_LENGTH);

            warn!(key = key, "MUUUXY_SERVER_KEY not set, using generated");

            key
        }
    };

    info!(key = server_key, "server key defined");

    let server_address = format!("{}:{}", server_host, server_port);

    let state = Arc::new(State::new(
        server_scheme,
        server_host,
        server_port,
        server_domain,
        server_proxy,
        server_key,
    ));

    let service = ServiceBuilder::new()
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET])
                .allow_headers(Any),
        )
        .layer(CompressionLayer::new())
        .layer(PropagateHeaderLayer::new(HeaderName::from_static(
            "x-request-id",
        )));

    let router = Router::new()
        .route("/healthz", get(healthz))
        .route("/proxy", get(proxy))
        .layer(Extension(state))
        .layer(service);

    info!("muuuxy server started");

    return serve(TcpListener::bind(server_address).await?, router).await;
}
