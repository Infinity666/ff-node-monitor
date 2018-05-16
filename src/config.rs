use rocket;
use rocket::fairing::{Fairing, AdHoc};

use toml;
use url::Url;
use url_serde;
use serde::Deserialize;
use serde::de::IntoDeserializer;

use db_conn;
use util;

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub instance_name: String,
    #[serde(with = "url_serde")]
    pub root_url: Url,
    pub email_from: String,
    pub postgres_url: String,
    #[serde(with = "util::hex_bytes")]
    pub action_signing_key: Vec<u8>,
}

impl Config {
    /// Create a `Config` instance from a rocket config table
    pub fn new(table: &rocket::config::Table) -> Self {
        let val = toml::value::Value::Table(table.clone());
        Self::deserialize(val.into_deserializer())
            .expect("app config table has missing or extra value")
    }
}

pub fn fairing(section: &'static str) -> impl Fairing {
    AdHoc::on_attach(move |rocket| {
        let config = {
            let config_table = rocket.config().get_table(section)
                .expect(format!("[{}] table in Rocket.toml missing or not a table", section).as_str());
            Config::new(config_table)
        };
        Ok(rocket
            .manage(db_conn::init_db_pool(config.postgres_url.as_str()))
            .manage(config))
    })
}