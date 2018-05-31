//  ff-node-monitor -- Monitoring for Freifunk nodes
//  Copyright (C) 2018  Ralf Jung <post AT ralfj DOT de>
//
//  This program is free software: you can redistribute it and/or modify
//  it under the terms of the GNU Affero General Public License as published by
//  the Free Software Foundation, either version 3 of the License, or
//  (at your option) any later version.
//
//  This program is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU Affero General Public License for more details.
//
//  You should have received a copy of the GNU Affero General Public License
//  along with this program.  If not, see <https://www.gnu.org/licenses/>.

use rocket::response::NamedFile;
use rocket::{State, request::Form};
use rocket_contrib::Template;

use diesel::prelude::*;
use failure::Error;
use lettre::{EmailTransport, SmtpTransport};
use rmp_serde::to_vec as serialize_to_vec;
use rmp_serde::from_slice as deserialize_from_slice;
use base64;

use std::path::{Path, PathBuf};
use std::io;

use db_conn::DbConn;
use action::*;
use models::*;
use config::{Config, Renderer};
use util::EmailBuilder;
use cron;

#[get("/")]
fn index(renderer: Renderer) -> Result<Template, Error> {
    renderer.render("index", json!({}))
}

#[derive(Serialize, FromForm)]
struct ListForm {
    email: String,
}

#[get("/list?<form>")]
fn list(form: ListForm, renderer: Renderer, db: DbConn) -> Result<Template, Error> {
    use schema::*;

    let watched_nodes = monitors::table
        .filter(monitors::email.eq(form.email.as_str()))
        .left_join(nodes::table.on(monitors::id.eq(nodes::id)))
        .order_by(monitors::id)
        .load::<(MonitorQuery, Option<NodeQuery>)>(&*db)?;
    let all_nodes = nodes::table
        .order_by(nodes::name)
        .load::<NodeQuery>(&*db)?;
    renderer.render("list", json!({
        "form": form,
        "watched_nodes": watched_nodes,
        "all_nodes": all_nodes,
    }))
}

#[post("/prepare_action", data = "<action>")]
fn prepare_action(
    action: Form<Action>,
    config: State<Config>,
    renderer: Renderer,
    email_builder: EmailBuilder,
) -> Result<Template, Error>
{
    let action = action.into_inner();

    // obtain bytes for signed action payload
    let signed_action = action.clone().sign(&config.secrets.action_signing_key);
    let signed_action = serialize_to_vec(&signed_action)?;
    let signed_action = base64::encode(&signed_action);

    // Generate email text
    let action_url = url_query!(config.urls.root.join("run_action")?,
        signed_action = signed_action);
    let list_url = url_query!(config.urls.root.join("list")?,
        email = action.email);
    let email_template = renderer.render("confirm_action", json!({
        "action": action,
        "action_url": action_url.as_str(),
        "list_url": list_url.as_str(),
    }))?;
    // Build and send email
    let email = email_builder.new(email_template)?
        .to(action.email.as_str())
        .build()?;
    let mut mailer = SmtpTransport::builder_unencrypted_localhost()?.build();
    mailer.send(&email)?;

    // Render
    let list_url = url_query!(config.urls.root.join("list")?,
        email = action.email);
    renderer.render("prepare_action", json!({
        "action": action,
        "list_url": list_url.as_str(),
    }))
}

#[derive(Serialize, FromForm)]
struct RunActionForm {
    signed_action: String,
}

#[get("/run_action?<form>")]
fn run_action(
    form: RunActionForm,
    db: DbConn,
    renderer: Renderer,
    config: State<Config>
) -> Result<Template, Error> {
    // Determine and verify action
    let action : Result<Action, Error> = do catch {
        let signed_action = base64::decode(form.signed_action.as_str())?;
        let signed_action: SignedAction = deserialize_from_slice(signed_action.as_slice())?;
        signed_action.verify(&config.secrets.action_signing_key)?
    };
    let action = match action {
        Ok(a) => a,
        Err(_) => {
            return renderer.render("action_error", json!({}))
        }
    };

    // Execute action
    let success = action.run(&*db)?;

    // Render
    let list_url = url_query!(config.urls.root.join("list")?,
        email = action.email);
    renderer.render("run_action", json!({
        "action": action,
        "list_url": list_url.as_str(),
        "success": success,
    }))
}

#[get("/cron")]
fn cron(
    db: DbConn,
    config: State<Config>,
    renderer: Renderer,
    email_builder: EmailBuilder,
) -> Result<(), Error> {
    cron::update_nodes(&*db, &*config, renderer, email_builder)?;
    Ok(())
}

#[get("/static/<file..>")]
fn static_file(file: PathBuf) -> Result<Option<NamedFile>, Error> {
    // Using Option<...> turns errors into 404
    Ok(match NamedFile::open(Path::new("static/").join(file)) {
        Ok(x) => Some(x),
        Err(ref x) if x.kind() == io::ErrorKind::NotFound => None,
        Err(x) => bail!(x),
    })
}

pub fn routes() -> Vec<::rocket::Route> {
    routes![index, list, prepare_action, run_action, cron, static_file]
}
