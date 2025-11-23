use std::{env, error::Error, fs::File, io::BufReader};

use actix_web::{
    App, HttpResponse, HttpServer, Responder, get,
    http::{StatusCode, header::ContentType},
    post, web,
};

use rustls::ServerConfig;

use crate::controller::Controller;

mod controller;

fn load_rustls_config() -> Result<ServerConfig, Box<dyn Error>> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .unwrap();

    let cert_path =
        env::var("GRAVIVOL_TLS_CERT_PATH").unwrap_or_else(|_| "/certs/cert.pem".to_string());
    let key_path =
        env::var("GRAVIVOL_TLS_KEY_PATH").unwrap_or_else(|_| "/certs/key.pem".to_string());
    let mut certs_file = BufReader::new(File::open(&cert_path)?);
    let mut key_file = BufReader::new(File::open(&key_path)?);

    // load TLS certs and key
    // to create a self-signed temporary cert for testing:
    // `openssl req -x509 -newkey rsa:4096 -nodes -keyout key.pem -out cert.pem -days 365 -subj '/CN=localhost'`
    let tls_certs = rustls_pemfile::certs(&mut certs_file).collect::<Result<Vec<_>, _>>()?;
    let tls_key = rustls_pemfile::pkcs8_private_keys(&mut key_file)
        .next()
        .expect("No key in PKCS#1 format found!")?;

    // set up TLS config options
    let tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(tls_certs, rustls::pki_types::PrivateKeyDer::Pkcs8(tls_key))?;

    Ok(tls_config)
}

#[post("/mutate")]
async fn mutate(req_body: String, controller: web::Data<Controller>) -> impl Responder {
    log::debug!("Got: {}", req_body);

    if let Ok(review) = serde_json::from_str(&req_body) {
        match controller.mutate(review) {
            Ok(response) => {
                log::debug!("Response is OK: {:?}", response);
                HttpResponse::Ok().json(response)
            }
            Err(err) => {
                log::error!("Respone error: {}", err);
                HttpResponse::build(StatusCode::BAD_REQUEST)
                    .insert_header(ContentType::html())
                    .body(err.to_string())
            }
        }
    } else {
        log::error!("Could not parse AdmissionReview JSON: {}", req_body);
        HttpResponse::build(StatusCode::BAD_REQUEST)
            .insert_header(ContentType::html())
            .body("Failed to parse AdmissionReview from JSON")
    }
}

#[get("/health")]
async fn health() -> impl Responder {
    "OK"
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    let tls_config = load_rustls_config().expect("Cannot load TLS config");

    let config = env::var("GRAVIVOL_CONFIG").unwrap_or_else(|_| "".to_string());
    log::info!("Got config: '{config}'");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(Controller::new(&config)))
            .service(mutate)
            .service(health)
    })
    .bind_rustls_0_23("[::]:8080", tls_config)?
    //.bind("[::]:8081")?
    .run()
    .await
}
