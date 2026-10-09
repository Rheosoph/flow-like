//! `valuable::Valuable` impls for types whose fields aren't themselves
//! `Valuable`: `Ipv4Addr`/`SocketAddr` render as their string form, `Duration`
//! as whole milliseconds under an `_ms`-suffixed field name.

use crate::gige::GigeConfig;
use crate::gige::discovery::{DiscoveredDevice, DiscoveryConfig, ForceIpConfig, NetworkAdapter};
use crate::gige::proto::bootstrap::DeviceInfo;
use crate::gige::stream::StreamConfig;

macro_rules! valuable_struct {
    (@val $local:ident plain)    => { valuable::Valuable::as_value($local) };
    (@val $local:ident ip)       => { valuable::Value::String(&$local) };
    (@val $local:ident addr)     => { valuable::Value::String(&$local) };
    (@val $local:ident opt_addr) => {
        $local.as_deref().map_or(valuable::Value::Unit, valuable::Value::String)
    };
    (@val $local:ident dur_ms)   => { valuable::Value::U64($local) };

    (@bind plain    $src:expr) => { &$src };
    (@bind ip       $src:expr) => { $src.to_string() };
    (@bind addr     $src:expr) => { $src.to_string() };
    (@bind opt_addr $src:expr) => { $src.map(|a| a.to_string()) };
    (@bind dur_ms   $src:expr) => { $src.as_millis() as u64 };

    (@fname $field:ident dur_ms) => { concat!(stringify!($field), "_ms") };
    (@fname $field:ident $k:ident) => { stringify!($field) };

    ($ty:ty, $name:literal { $( $field:ident : $kind:ident ),* $(,)? }) => {
        impl valuable::Valuable for $ty {
            fn as_value(&self) -> valuable::Value<'_> {
                valuable::Value::Structable(self)
            }
            fn visit(&self, visit: &mut dyn valuable::Visit) {
                $( let $field = valuable_struct!(@bind $kind self.$field); )*
                const FIELDS: &[valuable::NamedField<'static>] =
                    &[ $( valuable::NamedField::new(valuable_struct!(@fname $field $kind)) ),* ];
                let values = [ $( valuable_struct!(@val $field $kind) ),* ];
                visit.visit_named_fields(&valuable::NamedValues::new(FIELDS, &values));
            }
        }
        impl valuable::Structable for $ty {
            fn definition(&self) -> valuable::StructDef<'_> {
                const FIELDS: &[valuable::NamedField<'static>] =
                    &[ $( valuable::NamedField::new(valuable_struct!(@fname $field $kind)) ),* ];
                valuable::StructDef::new_static($name, valuable::Fields::Named(FIELDS))
            }
        }
    };
}

valuable_struct!(DeviceInfo, "DeviceInfo" {
    spec_version: plain,
    device_mode: plain,
    mac: plain,
    supported_ip_config: plain,
    current_ip_config: plain,
    ip: ip,
    subnet_mask: ip,
    gateway: ip,
    manufacturer: plain,
    model: plain,
    device_version: plain,
    manufacturer_info: plain,
    serial: plain,
    user_defined_name: plain,
});

valuable_struct!(NetworkAdapter, "NetworkAdapter" {
    name: plain,
    ip: ip,
    netmask: ip,
    broadcast: ip,
});

valuable_struct!(DiscoveredDevice, "DiscoveredDevice" {
    info: plain,
    from: addr,
    adapter: plain,
});

valuable_struct!(DiscoveryConfig, "DiscoveryConfig" {
    adapters: plain,
    recv_window: dur_ms,
    limited_broadcast: plain,
    source_port: plain,
    device_port: plain,
});

valuable_struct!(ForceIpConfig, "ForceIpConfig" {
    discovery: plain,
    ack_window: dur_ms,
});

valuable_struct!(GigeConfig, "GigeConfig" {
    addr: addr,
    local_addr: opt_addr,
    gvcp_timeout: dur_ms,
    retries: plain,
    heartbeat_timeout_ms: plain,
    exclusive: plain,
    event_capacity: plain,
    thread_cfg: plain,
});

valuable_struct!(StreamConfig, "StreamConfig" {
    channel: plain,
    payload_size: plain,
    n_buffers: plain,
    packet_size: plain,
    packet_delay: plain,
    resend: plain,
    initial_packet_timeout: dur_ms,
    packet_timeout: dur_ms,
    frame_retention: dur_ms,
    packet_request_ratio: plain,
    socket_buffer: plain,
    local_addr: opt_addr,
    thread_cfg: plain,
});
