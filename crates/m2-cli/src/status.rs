//! `openm2 status`: whether a server is up and speaks openm2. It asks the
//! auth server for its version and providers, as the login screen does, and
//! finishes a TLS handshake with the game server; the exit code says whether
//! both answered, for scripts and deploys (`docs/specs/client.md`).

use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use m2_core::ServerKind;

use crate::bot::ServerArgs;

/// Arguments of `openm2 status`.
#[derive(Debug, Args)]
pub struct StatusArgs {
    #[command(flatten)]
    server: ServerArgs,
}

/// Runs `status`: 0 when both servers answered, 1 otherwise.
pub fn run(mut args: StatusArgs) -> Result<ExitCode> {
    args.server.resolve()?;
    let net = args.server.net_config();
    let mut ok = true;

    let auth = format!("{}:{}", net.host, net.auth_port);
    match crate::servers::ask(&net) {
        Ok(answer) => {
            println!(
                "auth  {auth}  {}, protocol {}",
                answer.name, answer.protocol
            );
            let providers: Vec<String> = answer
                .providers
                .iter()
                .map(|provider| format!("{} ({})", provider.id, provider.name))
                .collect();
            println!("      providers: {}", providers.join(", "));
            if answer.key_sign_in {
                println!("      devices' keys: yes");
            }
        }
        Err(why) => {
            ok = false;
            println!("auth  {auth}  failed: {why}");
        }
    }

    let game = format!("{}:{}", net.host, net.game_port);
    match m2_net::probe(&net, ServerKind::Game) {
        Ok(addr) => println!("game  {game}  answered from {addr}, TLS 1.3 and openm2/1"),
        Err(why) => {
            ok = false;
            println!("game  {game}  failed: {why}");
        }
    }

    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
