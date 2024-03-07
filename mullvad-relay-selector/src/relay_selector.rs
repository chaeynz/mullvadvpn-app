//! The implementation of the relay selector.

mod detailer;
mod helpers;
mod matcher;
#[cfg(test)]
mod tests;

use chrono::{DateTime, Local};
use itertools::Itertools;
use once_cell::sync::Lazy;
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use matcher::{BridgeMatcher, RelayMatcher, WireguardMatcher};
use mullvad_types::{
    constraints::{Constraint, Intersection},
    custom_list::CustomListsSettings,
    endpoint::{MullvadEndpoint, MullvadWireguardEndpoint},
    location::{Coordinates, Location},
    relay_constraints::{
        BridgeSettings, BridgeSettingsFilter, BridgeState, InternalBridgeConstraints,
        ObfuscationSettings, OpenVpnConstraints, OpenVpnConstraintsFilter, RelayConstraintsFilter,
        RelayOverride, RelaySettings, ResolvedBridgeSettings, SelectedObfuscation,
        WireguardConstraints, WireguardConstraintsFilter,
    },
    relay_list::{Relay, RelayList},
    settings::Settings,
    CustomTunnelEndpoint,
};
use talpid_types::{
    net::{obfuscation::ObfuscatorConfig, proxy::CustomProxy, TransportProtocol, TunnelType},
    ErrorExt,
};

use crate::error::Error;
use crate::parsed_relays::ParsedRelays;

use self::{
    detailer::{OpenVpnDetailer, WireguardDetailer},
    matcher::AnyTunnelMatcher,
};

/// [`RETRY_ORDER`] defines an ordered set of relay parameters which the relay selector should prioritize on
/// successive connection attempts.
/// in successive retry attempts: https://linear.app/mullvad/issue/DES-543/optimize-order-of-connection-parameters-when-trying-to-connect
pub static RETRY_ORDER: Lazy<Vec<RelayConstraintsFilter>> = Lazy::new(|| {
    use mullvad_types::relay_constraints::builder::{any, openvpn, wireguard};
    vec![
        // 0
        any().build(),
        // 1
        wireguard::new().build(),
        // 2
        wireguard::new().port(443).build(),
        // 3
        wireguard::new()
            .ip_version(wireguard::IpVersion::V6)
            .build(),
        // 4
        openvpn::new()
            .transport_protocol(openvpn::TransportProtocol::Tcp)
            .port(443)
            .build(),
        // 5
        wireguard::new().udp2tcp().build(),
        // 6
        wireguard::new()
            .udp2tcp()
            .ip_version(wireguard::IpVersion::V6)
            .build(),
        // 7
        openvpn::new()
            .transport_protocol(openvpn::TransportProtocol::Tcp)
            .bridge()
            .build(),
    ]
});

#[derive(Clone)]
pub struct RelaySelector {
    config: Arc<Mutex<SelectorConfig>>,
    parsed_relays: Arc<Mutex<ParsedRelays>>,
}

#[derive(Clone)]
pub struct SelectorConfig {
    // Pure settings
    pub relay_settings: RelaySettings,
    pub custom_lists: CustomListsSettings,
    pub relay_overrides: Vec<RelayOverride>,
    // Wireguard specific data
    pub obfuscation_settings: ObfuscationSettings,
    // OpenVPN specific data
    pub bridge_state: BridgeState,
    pub bridge_settings: BridgeSettings,
}

impl Default for SelectorConfig {
    fn default() -> Self {
        let default_settings = Settings::default();
        SelectorConfig {
            relay_settings: default_settings.relay_settings,
            bridge_settings: default_settings.bridge_settings,
            obfuscation_settings: default_settings.obfuscation_settings,
            bridge_state: default_settings.bridge_state,
            custom_lists: default_settings.custom_lists,
            relay_overrides: default_settings.relay_overrides,
        }
    }
}

impl From<SelectorConfig> for RelayConstraintsFilter {
    /// Map user settings to [`RelayConstraintsFilter`].
    fn from(value: SelectorConfig) -> Self {
        /// Map the Wireguard-specific bits of `value` to [`WireguardConstraintsFilter`]
        fn wireguard_constraints(
            wireguard_constraints: WireguardConstraints,
            obfuscation_settings: ObfuscationSettings,
        ) -> WireguardConstraintsFilter {
            let WireguardConstraints {
                port,
                ip_version,
                use_multihop,
                entry_location,
            } = wireguard_constraints;
            WireguardConstraintsFilter {
                port,
                ip_version,
                use_multihop,
                entry_location,
                obfuscation: obfuscation_settings.selected_obfuscation,
                udp2tcp_port: Constraint::Only(obfuscation_settings.udp2tcp.clone()),
            }
        }

        /// Map the OpenVPN-specific bits of `value` to [`OpenVpnConstraintsFilter`]
        fn openvpn_constraints(
            openvpn_constraints: OpenVpnConstraints,
            bridge_state: BridgeState,
            bridge_settings: BridgeSettings,
        ) -> OpenVpnConstraintsFilter {
            OpenVpnConstraintsFilter {
                port: openvpn_constraints.port,
                bridge_settings: match bridge_state {
                    BridgeState::On => match bridge_settings.bridge_type {
                        mullvad_types::relay_constraints::BridgeType::Normal => Constraint::Only(
                            BridgeSettingsFilter::Normal(bridge_settings.normal.clone()),
                        ),
                        mullvad_types::relay_constraints::BridgeType::Custom => Constraint::Only(
                            BridgeSettingsFilter::Custom(bridge_settings.custom.clone()),
                        ),
                    },
                    BridgeState::Auto => Constraint::Only(BridgeSettingsFilter::Auto),
                    BridgeState::Off => Constraint::Only(BridgeSettingsFilter::Off),
                },
            }
        }

        match value.relay_settings {
            RelaySettings::CustomTunnelEndpoint(_) => panic!("Honestly don't know what to do"),
            RelaySettings::Normal(relay_constraints) => {
                let wireguard_constraints = wireguard_constraints(
                    relay_constraints.wireguard_constraints.clone(),
                    value.obfuscation_settings.clone(),
                );
                let openvpn_constraints = openvpn_constraints(
                    relay_constraints.openvpn_constraints,
                    value.bridge_state,
                    value.bridge_settings.clone(),
                );
                RelayConstraintsFilter {
                    location: relay_constraints.location.clone(),
                    providers: relay_constraints.providers.clone(),
                    ownership: relay_constraints.ownership,
                    tunnel_protocol: relay_constraints.tunnel_protocol,
                    wireguard_constraints,
                    openvpn_constraints,
                }
            }
        }
    }
}

/// The return type of `get_relay`.
#[derive(Clone, Debug)]
pub enum GetRelay {
    Wireguard {
        endpoint: MullvadEndpoint,
        exit: Relay,
        entry: Option<Relay>,
        obfuscator: Option<SelectedObfuscator>,
    },
    OpenVpn {
        endpoint: MullvadEndpoint,
        exit: Relay,
        bridge: Option<SelectedBridge>,
    },
    Custom(CustomTunnelEndpoint),
}

#[derive(Clone, Debug)]
pub enum SelectedBridge {
    Normal { settings: CustomProxy, relay: Relay },
    Custom(CustomProxy),
}

#[derive(Clone, Debug)]
pub struct SelectedObfuscator {
    pub config: ObfuscatorConfig,
    pub relay: Relay,
}

impl RelaySelector {
    /// Returns a new `RelaySelector` backed by relays cached on disk.
    pub fn new(
        config: SelectorConfig,
        resource_path: impl AsRef<Path>,
        cache_path: impl AsRef<Path>,
    ) -> Self {
        const DATE_TIME_FORMAT_STR: &str = "%Y-%m-%d %H:%M:%S%.3f";
        let unsynchronized_parsed_relays =
            ParsedRelays::from_file(&cache_path, &resource_path, &config.relay_overrides)
                .unwrap_or_else(|error| {
                    log::error!(
                        "{}",
                        error.display_chain_with_msg("Unable to load cached and bundled relays")
                    );
                    ParsedRelays::empty()
                });
        log::info!(
            "Initialized with {} cached relays from {}",
            unsynchronized_parsed_relays.relays().count(),
            DateTime::<Local>::from(unsynchronized_parsed_relays.last_updated())
                .format(DATE_TIME_FORMAT_STR)
        );

        RelaySelector {
            config: Arc::new(Mutex::new(config)),
            parsed_relays: Arc::new(Mutex::new(unsynchronized_parsed_relays)),
        }
    }

    pub fn from_list(config: SelectorConfig, relay_list: RelayList) -> Self {
        RelaySelector {
            parsed_relays: Arc::new(Mutex::new(ParsedRelays::from_relay_list(
                relay_list,
                SystemTime::now(),
                &config.relay_overrides,
            ))),
            config: Arc::new(Mutex::new(config)),
        }
    }

    pub fn set_config(&mut self, config: SelectorConfig) {
        self.set_overrides(&config.relay_overrides);
        let mut config_mutex = self.config.lock().unwrap();
        *config_mutex = config;
    }

    pub fn set_relays(&self, relays: RelayList) {
        let mut parsed_relays = self.parsed_relays.lock().unwrap();
        parsed_relays.update(relays);
    }

    fn set_overrides(&mut self, relay_overrides: &[RelayOverride]) {
        let mut parsed_relays = self.parsed_relays.lock().unwrap();
        parsed_relays.set_overrides(relay_overrides);
    }

    /// Returns all countries and cities. The cities in the object returned does not have any
    /// relays in them.
    pub fn get_relays(&mut self) -> RelayList {
        let parsed_relays = self.parsed_relays.lock().unwrap();
        parsed_relays.original_list().clone()
    }

    pub fn etag(&self) -> Option<String> {
        self.parsed_relays.lock().unwrap().etag()
    }

    pub fn last_updated(&self) -> SystemTime {
        self.parsed_relays.lock().unwrap().last_updated()
    }

    /// Returns a non-custom bridge based on the relay and bridge constraints, ignoring the bridge
    /// state.
    pub fn get_bridge_forced(&self) -> Option<CustomProxy> {
        let parsed_relays = &self.parsed_relays.lock().unwrap();
        let config = self.config.lock().unwrap();
        let near_location = match &config.relay_settings {
            RelaySettings::Normal(_) => {
                let user_preferences = RelayConstraintsFilter::from(config.clone());
                Self::get_relay_midpoint(parsed_relays, &user_preferences, &config)
            }
            _ => None,
        };
        let bridge_settings = &config.bridge_settings;
        let constraints = match bridge_settings.resolve() {
            Ok(ResolvedBridgeSettings::Normal(settings)) => InternalBridgeConstraints {
                location: settings.location.clone(),
                providers: settings.providers.clone(),
                ownership: settings.ownership,
                transport_protocol: Constraint::Only(TransportProtocol::Tcp),
            },
            _ => InternalBridgeConstraints {
                location: Constraint::Any,
                providers: Constraint::Any,
                ownership: Constraint::Any,
                transport_protocol: Constraint::Only(TransportProtocol::Tcp),
            },
        };

        let custom_lists = &config.custom_lists;
        Self::get_proxy_settings(parsed_relays, &constraints, near_location, custom_lists)
            .map(|(settings, _relay)| settings)
    }

    /// Returns a random relay and relay endpoint matching the current constraints defined by
    /// `retry_order`.
    ///
    /// See [`RETRY_ORDER`] for the current constraints.
    ///
    /// [`RETRY_ORDER`]: mullvad_relay_selector::relay_selector::RETRY_ORDER
    pub fn get_relay(&self, retry_attempt: usize) -> Result<GetRelay, Error> {
        self.get_relay_with_order(&RETRY_ORDER, retry_attempt)
    }

    /// Returns a random relay and relay endpoint matching the current constraints defined by
    /// `retry_order` corresponsing to `retry_attempt`.
    pub(crate) fn get_relay_with_order(
        &self,
        retry_order: &[RelayConstraintsFilter],
        retry_attempt: usize,
    ) -> Result<GetRelay, Error> {
        let config = self.config.lock().unwrap();

        // Short-ciruit if a custom tunnel endpoint is to be used - don't have to involve the
        // relay selector further!
        if let RelaySettings::CustomTunnelEndpoint(custom_relay) = &config.relay_settings {
            return Ok(GetRelay::Custom(custom_relay.clone()));
        }

        let parsed_relays = &self.parsed_relays.lock().unwrap();
        // Merge user preferences with the relay selector's default preferences.
        let user_preferences = RelayConstraintsFilter::from(config.clone());
        let constraints = retry_order
            .iter()
            .cycle()
            .filter_map(|constraint| constraint.clone().intersection(user_preferences.clone()))
            .nth(retry_attempt)
            .unwrap();

        Self::get_relay_inner(&constraints, parsed_relays, &config)
    }

    /// Returns random relay and relay endpoint matching `query`.
    pub fn get_relay_by_query(&self, query: RelayConstraintsFilter) -> Result<GetRelay, Error> {
        let parsed_relays = &self.parsed_relays.lock().unwrap();
        let config = self.config.lock().unwrap();
        Self::get_relay_inner(&query, parsed_relays, &config)
    }

    /// "Execute" the given query, yielding a final set of relays and/or bridges which the VPN traffic shall be routed through.
    ///
    /// # Parameters
    /// - `query`: Constraints that filter the available relays, such as geographic location or tunnel protocol.
    /// - `config`: Configuration settings that influence relay selection, including bridge state and custom lists.
    /// - `parsed_relays`: The complete set of parsed relays available for selection.
    ///
    /// # Returns
    /// * A randomly selected relay that meets the specified constraints (and a random bridge/entry relay if applicable).
    /// See [`GetRelay`] for more details.
    /// * An `Err` if no suitable relay is found
    /// * An `Err` if no suitable bridge is found
    fn get_relay_inner(
        query: &RelayConstraintsFilter,
        parsed_relays: &ParsedRelays,
        config: &SelectorConfig,
    ) -> Result<GetRelay, Error> {
        match query.tunnel_protocol {
            Constraint::Only(TunnelType::Wireguard) => {
                let (exit, entry) = if !query.wireguard_constraints.multihop() {
                    let exit =
                        Self::choose_relay(query, config, parsed_relays).ok_or(Error::NoRelay)?;
                    (exit, None)
                } else {
                    let (exit, entry) =
                        Self::get_wireguard_multihop_config(query, config, parsed_relays)?;
                    (exit, Some(entry))
                };

                let endpoint = {
                    let detailer = if let Some(ref entry) = entry {
                        WireguardDetailer::new(
                            query.wireguard_constraints.clone(),
                            exit.clone(),
                            parsed_relays.parsed_list().wireguard.clone(),
                        )
                        .set_entry(entry.clone())
                    } else {
                        WireguardDetailer::new(
                            query.wireguard_constraints.clone(),
                            exit.clone(),
                            parsed_relays.parsed_list().wireguard.clone(),
                        )
                    };
                    // TODO(markus): This is not the right error variant ..
                    detailer.to_endpoint().ok_or(Error::NoRelay)?
                };

                let obfuscator = match endpoint {
                    MullvadEndpoint::Wireguard(ref endpoint) => {
                        let obfuscator = {
                            let obfuscator_relay = entry.clone().unwrap_or(exit.clone());
                            let udp2tcp_ports =
                                parsed_relays.parsed_list().wireguard.udp2tcp_ports.clone();

                            Self::get_obfuscator(query, &udp2tcp_ports, &obfuscator_relay, endpoint)
                        };
                        obfuscator
                    }
                    _ => None,
                };

                Ok(GetRelay::Wireguard {
                    endpoint,
                    exit,
                    entry,
                    obfuscator,
                })
            }
            Constraint::Only(TunnelType::OpenVpn) => {
                let exit =
                    Self::choose_relay(query, config, parsed_relays).ok_or(Error::NoRelay)?;
                let detailer = OpenVpnDetailer::new(
                    query.openvpn_constraints.clone(),
                    exit.clone(),
                    parsed_relays.parsed_list().openvpn.clone(),
                );
                // TODO(markus): This is no the best error value in this situation..
                let endpoint = detailer.to_endpoint().ok_or(Error::NoRelay)?;
                let bridge = match endpoint {
                    MullvadEndpoint::OpenVpn(endpoint)
                        if helpers::should_use_bridge(
                            &query.openvpn_constraints.bridge_settings,
                        ) =>
                    {
                        let bridge_query =
                            &query.openvpn_constraints.bridge_settings.clone().unwrap();
                        Self::get_bridge(
                            bridge_query,
                            &exit,
                            &endpoint.protocol,
                            parsed_relays,
                            &config.custom_lists,
                        )?
                    }
                    _ => None,
                };
                Ok(GetRelay::OpenVpn {
                    endpoint,
                    exit,
                    bridge,
                })
            }
            Constraint::Any => {
                // Try Wireguard, then OpenVPN, then fail
                for tunnel_type in [TunnelType::Wireguard, TunnelType::OpenVpn] {
                    let mut new_constraints = query.clone();
                    new_constraints.tunnel_protocol = Constraint::Only(tunnel_type);
                    // If a suitable relay is found, short-circuit and return it
                    if let Ok(relay) =
                        Self::get_relay_inner(&new_constraints, parsed_relays, config)
                    {
                        return Ok(relay);
                    }
                }
                Err(Error::NoRelay)
            }
        }
    }

    /// Chooses a suitable relay from a set of parsed relays based on specified constraints and configuration.
    ///
    /// This function filters the available relays according to the given `RelayConstraintsFilter` and `SelectorConfig`,
    /// then selects one relay at random from the filtered list.
    ///
    /// # Returns
    /// A randomly selected relay that meets the specified constraints, or `None` if no suitable relay is found.
    fn choose_relay(
        query: &RelayConstraintsFilter,
        config: &SelectorConfig,
        parsed_relays: &ParsedRelays,
    ) -> Option<Relay> {
        // Filter among all valid relays
        let relays = Self::get_tunnel_endpoints(
            parsed_relays,
            query,
            config.bridge_state,
            &config.custom_lists,
        );
        // Pick one of the valid relays.
        helpers::pick_random_relay(&relays).cloned()
    }

    /// Returns a random relay and relay endpoint matching the given constraints and with
    /// preferences applied.
    #[cfg(target_os = "android")]
    #[cfg_attr(target_os = "android", allow(unused_variables))]
    fn get_tunnel_endpoints(
        parsed_relays: &ParsedRelays,
        relay_constraints: &RelayConstraints,
        bridge_state: BridgeState,
        custom_lists: &CustomListsSettings,
    ) -> Vec<Relay> {
        let relays = parsed_relays.relays();
        let matcher = WireguardMatcher::new_matcher(
            relay_constraints.clone(),
            relay_constraints.wireguard_constraints.clone(),
            parsed_relays.parsed_list().wireguard.clone(),
            custom_lists,
        );

        helpers::get_tunnel_endpoint_internal(&relays, &matcher)
    }

    #[cfg(not(target_os = "android"))]
    /// Returns a random relay and relay endpoint matching the given constraints and with
    /// preferences applied.
    fn get_tunnel_endpoints(
        parsed_relays: &ParsedRelays,
        relay_constraints: &RelayConstraintsFilter,
        bridge_state: BridgeState,
        custom_lists: &CustomListsSettings,
    ) -> Vec<Relay> {
        let relays = parsed_relays.relays();
        let matcher = RelayMatcher::new(
            relay_constraints.clone(),
            parsed_relays.parsed_list().openvpn.clone(),
            bridge_state,
            parsed_relays.parsed_list().wireguard.clone(),
            custom_lists,
        );
        matcher.filter_matching_relay_list(relays)
    }

    /// This function selects a valid entry and exit relay to be used in a multihop configuration.
    ///
    /// # Returns
    /// * An `Err` if no exit relay can be chosen
    /// * An `Err` if no entry relay can be chosen
    /// * An `Err` if the chosen entry and exit relays are the same
    /// * `Ok(exit, entry)` otherwise
    fn get_wireguard_multihop_config(
        query: &RelayConstraintsFilter,
        config: &SelectorConfig,
        parsed_relays: &ParsedRelays,
    ) -> Result<(Relay, Relay), Error> {
        // Here, we modify the original query just a bit.
        // The actual query for an exit relay is identical as for an exit relay, with the
        // exception that the location is different. It is simply the location as dictated by
        // the query's multihop constraint.
        let mut entry_relay_query = query.clone();
        entry_relay_query.location = query.wireguard_constraints.entry_location.clone();
        // After we have our two queries (one for the exit relay & one for the entry relay),
        // we can construct our two matchers:
        let wg_data = parsed_relays.parsed_list().wireguard.clone();
        let exit_matcher =
            WireguardMatcher::new_matcher(query.clone(), wg_data.clone(), &config.custom_lists);
        let entry_matcher = WireguardMatcher::new_matcher(
            entry_relay_query.clone(),
            wg_data.clone(),
            &config.custom_lists,
        );
        // .. and query for all exit & entry candidates! All candidates are needed for the next step.
        let exit_candidates = exit_matcher.filter_matching_relay_list(parsed_relays.relays());
        let entry_candidates = entry_matcher.filter_matching_relay_list(parsed_relays.relays());

        // This algorithm gracefully handles a particular edge case that arise when a constraint on
        // the exit relay is more specific than on the entry relay which forces the relay selector
        // to choose one specific relay. The relay selector could end up selecting that specific
        // relay as the entry relay, thus leaving no remaining exit relay candidates or vice versa.
        let (exit, entry) = match (exit_candidates.as_slice(), entry_candidates.as_slice()) {
            ([exit], [entry]) if exit == entry => None,
            (exits, [entry]) if exits.contains(entry) => {
                let exit = helpers::random(exits, entry).ok_or(Error::NoRelay)?;
                Some((exit, entry))
            }
            ([exit], entrys) if entrys.contains(exit) => {
                let entry = helpers::random(entrys, exit).ok_or(Error::NoRelay)?;
                Some((exit, entry))
            }
            (exits, entrys) => {
                let exit = helpers::pick_random_relay(exits).ok_or(Error::NoRelay)?;
                let entry = helpers::random(entrys, exit).ok_or(Error::NoRelay)?;
                Some((exit, entry))
            }
        }
        .ok_or(Error::NoRelay)?;

        Ok((exit.clone(), entry.clone()))
    }

    pub fn get_obfuscator(
        query: &RelayConstraintsFilter,
        udp2tcp_ports: &[u16],
        relay: &Relay,
        endpoint: &MullvadWireguardEndpoint,
    ) -> Option<SelectedObfuscator> {
        match query.wireguard_constraints.obfuscation {
            SelectedObfuscation::Off | SelectedObfuscation::Auto => None,
            SelectedObfuscation::Udp2Tcp => helpers::get_udp2tcp_obfuscator(
                &query.wireguard_constraints.udp2tcp_port,
                udp2tcp_ports,
                relay.clone(),
                endpoint,
            ),
        }
    }

    /// Selects a suitable bridge based on the specified settings, relay information, and transport protocol.
    ///
    /// # Parameters
    /// - `query`: The filter criteria for selecting a bridge.
    /// - `relay`: Information about the current relay, including its location.
    /// - `protocol`: The transport protocol (TCP or UDP) in use.
    /// - `parsed_relays`: A structured representation of all available relays.
    /// - `custom_lists`: User-defined or application-specific settings that may influence bridge selection.
    ///
    /// # Returns
    /// * On success, returns an `Option` containing the selected bridge, if one is found. Returns `None` if no suitable bridge meets the criteria.
    /// * `Error::NoBridge` if attempting to use OpenVPN bridges over UDP, as this is unsupported.
    /// * `Error::NoRelay` if `relay` does not have a location set.
    fn get_bridge(
        query: &BridgeSettingsFilter,
        relay: &Relay,
        protocol: &TransportProtocol,
        parsed_relays: &ParsedRelays,
        custom_lists: &CustomListsSettings,
    ) -> Result<Option<SelectedBridge>, Error> {
        match protocol {
            TransportProtocol::Udp => {
                log::error!("Can not use OpenVPN bridges over UDP");
                Err(Error::NoBridge)
            }
            TransportProtocol::Tcp => {
                let location = relay.location.as_ref().ok_or(Error::NoRelay)?;
                Ok(Self::get_bridge_for(
                    query,
                    location,
                    // FIXME: This is temporary while talpid-core only supports TCP proxies
                    TransportProtocol::Tcp,
                    parsed_relays,
                    custom_lists,
                ))
            }
        }
    }

    fn get_bridge_for(
        query: &BridgeSettingsFilter,
        location: &Location,
        transport_protocol: TransportProtocol,
        parsed_relays: &ParsedRelays,
        custom_lists: &CustomListsSettings,
    ) -> Option<SelectedBridge> {
        match query {
            BridgeSettingsFilter::Normal(settings) => {
                let bridge_constraints = InternalBridgeConstraints {
                    location: settings.location.clone(),
                    providers: settings.providers.clone(),
                    ownership: settings.ownership,
                    transport_protocol: Constraint::Only(transport_protocol),
                };

                Self::get_proxy_settings(
                    parsed_relays,
                    &bridge_constraints,
                    Some(location),
                    custom_lists,
                )
                .map(|(settings, relay)| SelectedBridge::Normal { settings, relay })
            }
            BridgeSettingsFilter::Custom(settings) => settings.clone().map(SelectedBridge::Custom),
            BridgeSettingsFilter::Off | BridgeSettingsFilter::Auto => None,
        }
    }

    /// Try to get a bridge that matches the given `constraints`.
    ///
    /// The connection details are returned alongside the relay hosting the bridge.
    fn get_proxy_settings<T: Into<Coordinates>>(
        parsed_relays: &ParsedRelays,
        constraints: &InternalBridgeConstraints,
        location: Option<T>,
        custom_lists: &CustomListsSettings,
    ) -> Option<(CustomProxy, Relay)> {
        let matcher = BridgeMatcher::new_matcher(constraints.clone(), custom_lists);
        let relays = matcher.filter_matching_relay_list(parsed_relays.relays());

        let relay = match location {
            Some(location) => Self::get_proximate_bridge(relays, location),
            None => helpers::pick_random_relay(&relays).cloned(),
        }?;

        let bridge = &parsed_relays.parsed_list().bridge;
        helpers::pick_random_bridge(bridge, &relay).map(|bridge| (bridge, relay.clone()))
    }

    /// Try to get a bridge which is close to `location`.
    fn get_proximate_bridge<T: Into<Coordinates>>(
        relays: Vec<Relay>,
        location: T,
    ) -> Option<Relay> {
        /// Minimum number of bridges to keep for selection when filtering by distance.
        const MIN_BRIDGE_COUNT: usize = 5;
        /// Max distance of bridges to consider for selection (km).
        const MAX_BRIDGE_DISTANCE: f64 = 1500f64;
        let location = location.into();

        #[derive(Clone)]
        struct RelayWithDistance {
            relay: Relay,
            distance: f64,
        }

        // Filter out all candidate bridges.
        let matching_relays: Vec<RelayWithDistance> = relays
            .into_iter()
            .map(|relay| RelayWithDistance {
                distance: relay.location.as_ref().unwrap().distance_from(&location),
                relay,
            })
            .sorted_unstable_by_key(|relay| relay.distance as usize)
            .take(MIN_BRIDGE_COUNT)
            .filter(|relay| relay.distance <= MAX_BRIDGE_DISTANCE)
            .collect();

        // Calculate the maximum distance from `location` among the candidates.
        let greatest_distance: f64 = matching_relays
            .iter()
            .map(|relay| relay.distance)
            .reduce(f64::max)?;
        // Define the weight function to prioritize bridges which are closer to `location`.
        let weight_fn = |relay: &RelayWithDistance| 1 + (greatest_distance - relay.distance) as u64;

        helpers::pick_random_relay_fn(&matching_relays, weight_fn)
            .cloned()
            .map(|relay_with_distance| relay_with_distance.relay)
    }

    /// Returns the average location of relays that match the given constraints.
    /// This returns `None` if the location is [`Constraint::Any`] or if no
    /// relays match the constraints.
    fn get_relay_midpoint(
        parsed_relays: &ParsedRelays,
        constraints: &RelayConstraintsFilter,
        config: &SelectorConfig,
    ) -> Option<Coordinates> {
        if constraints.location.is_any() {
            return None;
        }
        let (openvpn_data, wireguard_data) = (
            parsed_relays.parsed_list().openvpn.clone(),
            parsed_relays.parsed_list().wireguard.clone(),
        );

        let matcher = RelayMatcher::new(
            constraints.clone(),
            openvpn_data,
            config.bridge_state,
            wireguard_data,
            &config.custom_lists.clone(),
        );

        Self::get_relay_midpoint_inner(parsed_relays, matcher)
    }

    fn get_relay_midpoint_inner(
        parsed_relays: &ParsedRelays,
        matcher: RelayMatcher<AnyTunnelMatcher>,
    ) -> Option<Coordinates> {
        use std::ops::Not;
        let matching_locations: Vec<Location> = matcher
            .filter_matching_relay_list(parsed_relays.relays())
            .into_iter()
            .filter_map(|relay| relay.location)
            .unique_by(|location| location.city.clone())
            .collect();

        matching_locations
            .is_empty()
            .not()
            .then(|| Coordinates::midpoint(&matching_locations))
    }
}
