//! Fetches and prints the full relay list in JSON.
//! Used by the installer artifact packer to bundle the latest available
//! relay list at the time of creating the installer.

use mullvad_api::{self, connection_mode::DirectConnectionModeRepeater, rest, RelayListProxy};
use std::process;
use talpid_types::ErrorExt;

#[tokio::main]
async fn main() {
    let runtime = mullvad_api::Runtime::new(tokio::runtime::Handle::current())
        .expect("Failed to load runtime");

    let direct_repeater = DirectConnectionModeRepeater::new();
    let connection_mode_handle: mullvad_api::ConnectionModeActorHandle =
        mullvad_api::ConnectionModeActor::new(Box::new(direct_repeater));
    let relay_list_request =
        RelayListProxy::new(runtime.mullvad_rest_handle(connection_mode_handle).await)
            .relay_list(None)
            .await;

    let relay_list = match relay_list_request {
        Ok(relay_list) => relay_list,
        Err(rest::Error::TimeoutError) => {
            eprintln!("Request timed out");
            process::exit(2);
        }
        Err(e @ rest::Error::DeserializeError(_)) => {
            eprintln!(
                "{}",
                e.display_chain_with_msg("Failed to deserialize relay list")
            );
            process::exit(3);
        }
        Err(e) => {
            eprintln!("{}", e.display_chain_with_msg("Failed to fetch relay list"));
            process::exit(1);
        }
    };
    println!("{}", serde_json::to_string_pretty(&relay_list).unwrap());
}
