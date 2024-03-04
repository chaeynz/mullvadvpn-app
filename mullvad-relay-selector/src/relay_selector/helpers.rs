//! This module contains various helper functions for the relay selector implementation.

// TODO(markus): Put all functions which does not use the RelaySelector/`self` paramter here.

use std::net::SocketAddr;

use mullvad_types::{
    constraints::Constraint,
    endpoint::MullvadWireguardEndpoint,
    relay_constraints::Udp2TcpObfuscationSettings,
    relay_list::{BridgeEndpointData, Relay, RelayEndpointData, WireguardEndpointData},
};
use rand::{seq::SliceRandom, thread_rng, Rng};
use talpid_types::net::{obfuscation::ObfuscatorConfig, proxy::CustomProxy};

use super::matcher::WireguardMatcher;
use crate::{
    constants::{WIREGUARD_EXIT_IP_VERSION, WIREGUARD_EXIT_PORT},
    SelectedObfuscator, SelectorConfig,
};

/// Picks a relay using [Self::pick_random_relay_fn], using the `weight` member of each relay
/// as the weight function.
pub fn pick_random_relay(relays: &[Relay]) -> Option<&Relay> {
    pick_random_relay_fn(relays, |relay| relay.weight)
}

/// Pick a random relay from the given slice. Will return `None` if the given slice is empty.
/// If all of the relays have a weight of 0, one will be picked at random without bias,
/// otherwise roulette wheel selection will be used to pick only relays with non-zero
/// weights.
pub fn pick_random_relay_fn<RelayType>(
    relays: &[RelayType],
    weight_fn: impl Fn(&RelayType) -> u64,
) -> Option<&RelayType> {
    let total_weight: u64 = relays.iter().map(&weight_fn).sum();
    let mut rng = thread_rng();
    if total_weight == 0 {
        relays.choose(&mut rng)
    } else {
        // Pick a random number in the range 1..=total_weight. This choses the relay with a
        // non-zero weight.
        let mut i: u64 = rng.gen_range(1..=total_weight);
        Some(
            relays
                .iter()
                .find(|relay| {
                    i = i.saturating_sub(weight_fn(relay));
                    i == 0
                })
                .expect("At least one relay must've had a weight above 0"),
        )
    }
}

/// Picks a random bridge from a relay.
/// TODO(markus): Rip out state/RNG?
pub fn pick_random_bridge(data: &BridgeEndpointData, relay: &Relay) -> Option<CustomProxy> {
    if relay.endpoint_data != RelayEndpointData::Bridge {
        return None;
    }
    let shadowsocks_endpoint = data.shadowsocks.choose(&mut rand::thread_rng());
    if let Some(shadowsocks_endpoint) = shadowsocks_endpoint {
        log::info!(
            "Selected Shadowsocks bridge {} at {}:{}/{}",
            relay.hostname,
            relay.ipv4_addr_in,
            shadowsocks_endpoint.port,
            shadowsocks_endpoint.protocol
        );
    }
    shadowsocks_endpoint
        .map(|endpoint_data| endpoint_data.to_proxy_settings(relay.ipv4_addr_in.into()))
}

pub fn wireguard_exit_matcher(wg: WireguardEndpointData) -> WireguardMatcher {
    let mut tunnel = WireguardMatcher::from_endpoint(wg);
    tunnel.ip_version = WIREGUARD_EXIT_IP_VERSION;
    tunnel.port = WIREGUARD_EXIT_PORT;
    tunnel
}

pub fn get_udp2tcp_obfuscator(
    udp2tcp_ports: &[u16],
    obfuscation_settings: &Udp2TcpObfuscationSettings,
    relay: &Relay,
    endpoint: &MullvadWireguardEndpoint,
) -> Option<SelectedObfuscator> {
    let udp2tcp_endpoint = if obfuscation_settings.port.is_only() {
        udp2tcp_ports
            .iter()
            .find(|&candidate| obfuscation_settings.port == Constraint::Only(*candidate))
    } else {
        // Just return a 'random' port
        // TODO(markus): Can this randomness be pushsed up the stack?
        udp2tcp_ports.choose(&mut thread_rng())
    };

    udp2tcp_endpoint
        .map(|udp2tcp_endpoint| ObfuscatorConfig::Udp2Tcp {
            endpoint: SocketAddr::new(endpoint.peer.endpoint.ip(), *udp2tcp_endpoint),
        })
        .map(|config| SelectedObfuscator {
            config,
            relay: relay.clone(),
        })
}

// TODO(markus): This is not enough, right?
pub const fn should_use_bridge(config: &SelectorConfig) -> bool {
    use mullvad_types::relay_constraints::BridgeState;
    match config.bridge_state {
        BridgeState::On => true,
        BridgeState::Off => false,
        // TODO(markus): This should really be expressed as a constraint ..
        BridgeState::Auto => false,
    }
}
