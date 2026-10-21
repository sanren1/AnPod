use serde::Serialize;
use std::net::Ipv4Addr;

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Ipv4Interface {
    pub(crate) index: u32,
    pub(crate) name: String,
    pub(crate) address: Ipv4Addr,
    pub(crate) prefix_length: u8,
    pub(crate) gateways: Vec<Ipv4Addr>,
}

impl Ipv4Interface {
    pub(crate) fn contains(&self, address: Ipv4Addr) -> bool {
        let mask = prefix_mask(self.prefix_length);
        u32::from(self.address) & mask == u32::from(address) & mask
    }

    pub(crate) fn directed_broadcast(&self) -> Option<Ipv4Addr> {
        (1..=30)
            .contains(&self.prefix_length)
            .then(|| Ipv4Addr::from(u32::from(self.address) | !prefix_mask(self.prefix_length)))
    }
}

fn prefix_mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix.min(32))
    }
}

#[cfg(windows)]
pub(crate) fn enumerate_private_ipv4() -> Result<Vec<Ipv4Interface>, String> {
    use std::mem;
    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR},
            NetworkManagement::{
                IpHelper::{
                    GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_INCLUDE_PREFIX,
                    IF_TYPE_SOFTWARE_LOOPBACK, IP_ADAPTER_ADDRESSES_LH,
                },
                Ndis::IfOperStatusUp,
            },
            Networking::WinSock::{AF_INET, SOCKADDR_IN, SOCKET_ADDRESS},
        },
    };

    unsafe fn address_v4(address: &SOCKET_ADDRESS) -> Option<Ipv4Addr> {
        if address.lpSockaddr.is_null()
            || address.iSockaddrLength < mem::size_of::<SOCKADDR_IN>() as i32
        {
            return None;
        }
        let socket_address = unsafe { &*address.lpSockaddr.cast::<SOCKADDR_IN>() };
        if socket_address.sin_family != AF_INET {
            return None;
        }
        Some(Ipv4Addr::from(unsafe {
            socket_address.sin_addr.S_un.S_addr.to_ne_bytes()
        }))
    }

    let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_INCLUDE_PREFIX;
    let mut size = 0u32;
    let status = unsafe { GetAdaptersAddresses(AF_INET.0 as u32, flags, None, None, &mut size) };
    if status != ERROR_BUFFER_OVERFLOW.0 && status != NO_ERROR.0 {
        return Err(format!(
            "Unable to enumerate Windows IPv4 interfaces ({status})."
        ));
    }
    if size == 0 {
        return Ok(Vec::new());
    }

    let mut storage = vec![0usize; (size as usize).div_ceil(mem::size_of::<usize>())];
    let adapters = storage.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    let status =
        unsafe { GetAdaptersAddresses(AF_INET.0 as u32, flags, None, Some(adapters), &mut size) };
    if status != NO_ERROR.0 {
        return Err(format!(
            "Unable to read Windows IPv4 interface details ({status})."
        ));
    }

    let mut interfaces = Vec::new();
    let mut adapter = adapters;
    while !adapter.is_null() {
        let current = unsafe { &*adapter };
        if current.OperStatus == IfOperStatusUp && current.IfType != IF_TYPE_SOFTWARE_LOOPBACK {
            let index = unsafe { current.Anonymous1.Anonymous.IfIndex };
            let name = unsafe { PWSTR(current.FriendlyName.0).to_string() }
                .map_err(|_| "Unable to read a Windows interface name.".to_string())?;
            let mut unicast = current.FirstUnicastAddress;
            while !unicast.is_null() {
                let item = unsafe { &*unicast };
                if item.OnLinkPrefixLength != 0 {
                    if let Some(address) = unsafe { address_v4(&item.Address) }
                        .filter(|address| address.is_private() && !address.is_link_local())
                    {
                        let mut gateways = Vec::new();
                        let mut gateway = current.FirstGatewayAddress;
                        while !gateway.is_null() {
                            if let Some(address) = unsafe { address_v4(&(*gateway).Address) }
                                .filter(|address| {
                                    !address.is_unspecified() && !address.is_multicast()
                                })
                            {
                                if !gateways.contains(&address) {
                                    gateways.push(address);
                                }
                            }
                            gateway = unsafe { (*gateway).Next };
                        }
                        interfaces.push(Ipv4Interface {
                            index,
                            name: name.clone(),
                            address,
                            prefix_length: item.OnLinkPrefixLength.min(32),
                            gateways,
                        });
                    }
                }
                unicast = item.Next;
            }
        }
        adapter = current.Next;
    }
    Ok(interfaces)
}

#[cfg(not(windows))]
pub(crate) fn enumerate_private_ipv4() -> Result<Vec<Ipv4Interface>, String> {
    Err("IPv4 interface enumeration is available only on Windows.".into())
}

#[cfg(test)]
pub(crate) fn interface_for_test(address: Ipv4Addr, prefix_length: u8) -> Ipv4Interface {
    Ipv4Interface {
        index: 1,
        name: "test".into(),
        address,
        prefix_length,
        gateways: Vec::new(),
    }
}
