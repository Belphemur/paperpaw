//! The network over Wi-Fi: smoltcp (a TCP/IP stack made for devices
//! without an operating system) on the radio's station interface. It is
//! polled from the main loop (PaperPaw's firmware runs no async code). Its
//! address, router and name servers come by DHCP. What goes over it is in
//! `http` (requests), `serve` (servers) and `sockets` (plugins' own
//! connections and broadcasts).
//!
//! In the device's own network (an access point, [`Net::hosted`]) its
//! address is fixed and it gives the phones that join theirs
//! (`paperpaw_x4::dhcp`).

use alloc::vec::Vec;
use core::task::{Context, Waker};

use embassy_net_driver::Driver;
use esp_radio::wifi::Interface;
use smoltcp::iface::{Config, Interface as Stack, SocketHandle, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities, Medium};
use smoltcp::socket::{dhcpv4, dns, tcp, udp};
use smoltcp::time::Instant;
use smoltcp::wire::{DnsQueryType, EthernetAddress, HardwareAddress, IpAddress, IpCidr};

/// Without an address this long after joining a network, DHCP asks anew.
const DHCP_AGAIN_MS: u64 = 10_000;
/// Name lookups at a time (requests, plugins' sockets, the clock's).
const MAX_QUERIES: usize = 4;

/// The bytes a TCP connection buffers: received, and to send. The window
/// it offers is what it can receive, so more is faster and takes memory.
#[derive(Debug, Clone, Copy)]
pub struct Buffers {
    pub receive: usize,
    pub send: usize,
}

/// The number of a [`Net`], new each time the radio comes on: what was
/// opened on an earlier one is gone.
static mut GENERATION: u32 = 0;

/// A socket of one [`Net`]. Kept past the radio's restart, it is an
/// earlier one's: the Net passes over it (`paperpaw_x4::generation`), a
/// connection gone.
pub type Handle = paperpaw_x4::generation::Tagged<SocketHandle>;
/// A name lookup of one [`Net`], passed over by a later one as [`Handle`].
pub type Query = paperpaw_x4::generation::Tagged<dns::QueryHandle>;

#[derive(Debug)]
pub enum Error {
    /// No address yet (DHCP has not answered).
    Offline,
    /// The name could not be resolved.
    Name,
    /// The connection could not be opened.
    Connection,
    /// Not enough memory for it.
    Memory,
}

/// `len` zero bytes, or [`Error::Memory`]: a connection's buffers are
/// large, and memory may be short (or in pieces) at any time.
fn zeroed(len: usize) -> Result<Vec<u8>, Error> {
    let mut buf = Vec::new();
    buf.try_reserve_exact(len).map_err(|_| Error::Memory)?;
    buf.resize(len, 0);
    Ok(buf)
}

fn now() -> Instant {
    Instant::from_millis(
        esp_hal::time::Instant::now()
            .duration_since_epoch()
            .as_millis() as i64,
    )
}

/// The radio's interface as smoltcp wants a device.
struct Phy(Interface);

/// The radio's tokens for smoltcp.
struct Rx<T>(T);
struct Tx<T>(T);

impl<T: embassy_net_driver::RxToken> smoltcp::phy::RxToken for Rx<T> {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        self.0.consume(|buf| f(buf))
    }
}

impl<T: embassy_net_driver::TxToken> smoltcp::phy::TxToken for Tx<T> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        self.0.consume(len, f)
    }
}

impl Device for Phy {
    type RxToken<'a> = Rx<<Interface as Driver>::RxToken<'a>>;
    type TxToken<'a> = Tx<<Interface as Driver>::TxToken<'a>>;

    fn receive(&mut self, _: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        // Polled: nothing to wake.
        let mut cx = Context::from_waker(Waker::noop());
        Driver::receive(&mut self.0, &mut cx).map(|(rx, tx)| (Rx(rx), Tx(tx)))
    }

    fn transmit(&mut self, _: Instant) -> Option<Self::TxToken<'_>> {
        let mut cx = Context::from_waker(Waker::noop());
        Driver::transmit(&mut self.0, &mut cx).map(Tx)
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let theirs = Driver::capabilities(&self.0);
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = theirs.max_transmission_unit;
        caps.max_burst_size = theirs.max_burst_size;
        caps
    }
}

pub struct Net {
    phy: Phy,
    stack: Stack,
    sockets: SocketSet<'static>,
    /// Asks for the address; none in the device's own network.
    dhcp: Option<SocketHandle>,
    /// Gives the addresses in the device's own network: its socket.
    server: Option<(SocketHandle, paperpaw_x4::dhcp::Server)>,
    /// Created once DHCP named the name servers.
    dns: Option<SocketHandle>,
    /// The device's name as the router has it (DHCP's option 12): also
    /// the label of its `.local` name.
    label: Option<&'static [u8]>,
    /// Answers for `<label>.local` (multicast DNS): its socket, once the
    /// network gave an address.
    names: Option<(SocketHandle, paperpaw_x4::mdns::Responder)>,
    /// The address DHCP gave, with its network's size.
    address: Option<smoltcp::wire::Ipv4Cidr>,
    /// The next local port for a connection.
    port: u16,
    /// When DHCP was last (re)started while joined without an address.
    asked: Option<Instant>,
    generation: u32,
    /// What an earlier Net's handles reach: a TCP connection closed and a
    /// UDP socket not bound, both without buffers, made when first used.
    gone: Option<alloc::boxed::Box<(tcp::Socket<'static>, udp::Socket<'static>)>>,
}

impl Net {
    pub fn new(interface: Interface) -> Self {
        let mut net = Self::bare(interface);
        net.dhcp = Some(net.sockets.add(dhcpv4::Socket::new()));
        net
    }

    /// The name the router lists the device by (DHCP's option 12,
    /// `host`), from the next time DHCP asks (joining a network).
    pub fn set_host_name(&mut self, host: &'static [smoltcp::wire::DhcpOption<'static>]) {
        self.label = host.first().map(|option| option.data);
        if let Some(handle) = self.dhcp {
            self.sockets
                .get_mut::<dhcpv4::Socket<'static>>(handle)
                .set_outgoing_options(host);
        }
    }

    /// The device's own network, on the access point's interface: its
    /// address `address` (the network is its /24), addresses given to
    /// those that join. `None` without memory for the server's socket.
    pub fn hosted(interface: Interface, address: [u8; 4]) -> Option<Self> {
        use paperpaw_x4::dhcp;
        let mut net = Self::bare(interface);
        let cidr = smoltcp::wire::Ipv4Cidr::new(address.into(), 24);
        net.stack.update_ip_addrs(|addrs| {
            let _ = addrs.push(IpCidr::Ipv4(cidr));
        });
        net.address = Some(cidr);
        let handle = net.open_udp(4, 4 * 600).ok()?;
        let socket = net.udp(handle);
        socket.close();
        socket.bind(dhcp::SERVER_PORT).ok()?;
        net.server = Some((handle.of(net.generation)?, dhcp::Server::new(address)));
        Some(net)
    }

    /// The stack on `interface`, without an address yet.
    fn bare(interface: Interface) -> Self {
        let mut phy = Phy(interface);
        let mac = EthernetAddress(phy.0.mac_address());
        let rng = esp_hal::rng::Rng::new();
        let mut config = Config::new(HardwareAddress::Ethernet(mac));
        config.random_seed = u64::from(rng.random()) << 32 | u64::from(rng.random());
        let stack = Stack::new(config, &mut phy, now());
        let sockets = SocketSet::new(Vec::new());
        // SAFETY: one core, used from the main loop only.
        let generation = unsafe {
            let next = &mut *core::ptr::addr_of_mut!(GENERATION);
            *next = next.wrapping_add(1);
            *next
        };
        Self {
            phy,
            stack,
            sockets,
            dhcp: None,
            server: None,
            dns: None,
            label: None,
            names: None,
            address: None,
            // In the range for ports a client picks.
            port: 49152 + (rng.random() % 16000) as u16,
            asked: None,
            generation,
            gone: None,
        }
    }

    /// Which network this is (see [`GENERATION`]).
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// The local network's broadcast address, once there is an address.
    pub fn broadcast_address(&self) -> Option<IpAddress> {
        self.address?.broadcast().map(IpAddress::Ipv4)
    }

    /// The address DHCP gave, if it did yet.
    pub fn address(&self) -> Option<smoltcp::wire::Ipv4Cidr> {
        self.address
    }

    /// The link came up (a network was joined): DHCP asks at once, not
    /// after the waits it grew while nothing answered. An address kept
    /// from before is not used until DHCP gives it again: reported online
    /// with it, a request made then went out on an address DHCP was about
    /// to take back.
    pub fn joined(&mut self) {
        if let Some(dhcp) = self.dhcp {
            self.sockets.get_mut::<dhcpv4::Socket<'_>>(dhcp).reset();
            self.asked = Some(now());
            if self.address.is_some() {
                log::info!("net: asking DHCP for the address again");
                self.forget_address();
            }
        }
    }

    /// Sends and receives what is due, and takes what DHCP says.
    pub fn poll(&mut self) {
        // No answer long after joining (seen once after a start): ask anew.
        if let Some(asked) = self.asked
            && self.address.is_none()
            && now() - asked > smoltcp::time::Duration::from_millis(DHCP_AGAIN_MS)
        {
            log::info!("net: asking DHCP again");
            self.joined();
        }
        self.stack.poll(now(), &mut self.phy, &mut self.sockets);
        self.serve_addresses();
        self.serve_names();
        let Some(dhcp) = self.dhcp else {
            return;
        };
        let event = self.sockets.get_mut::<dhcpv4::Socket<'_>>(dhcp).poll();
        match event {
            None => {}
            Some(dhcpv4::Event::Configured(config)) => {
                log::info!("net: address {}", config.address);
                self.address = Some(config.address);
                self.stack.update_ip_addrs(|addrs| {
                    addrs.clear();
                    let _ = addrs.push(IpCidr::Ipv4(config.address));
                });
                let local = config.address.address().octets();
                match config.router {
                    Some(router) => {
                        let _ = self.stack.routes_mut().add_default_ipv4_route(router);
                    }
                    None => {
                        self.stack.routes_mut().remove_default_ipv4_route();
                    }
                }
                let servers: Vec<IpAddress> = config
                    .dns_servers
                    .iter()
                    .map(|&s| IpAddress::Ipv4(s))
                    .collect();
                match self.dns {
                    Some(handle) => self
                        .sockets
                        .get_mut::<dns::Socket<'_>>(handle)
                        .update_servers(&servers),
                    None => {
                        let socket = dns::Socket::new(
                            &servers,
                            (0..MAX_QUERIES).map(|_| None).collect::<Vec<_>>(),
                        );
                        self.dns = Some(self.sockets.add(socket));
                    }
                }
                self.answer_for_name(local);
            }
            Some(dhcpv4::Event::Deconfigured) => {
                log::info!("net: address gone");
                self.forget_address();
            }
        }
    }

    /// The link went (out of reach, the router restarted): the address
    /// DHCP gave goes with it, so the network counts as joined only once
    /// DHCP gave one again, and what was open on the old one ends then
    /// (requests see no address) rather than waiting on a link gone.
    pub fn link_lost(&mut self) {
        if self.dhcp.is_some() && self.address.is_some() {
            log::info!("net: address let go with the link");
            self.forget_address();
        }
    }

    fn forget_address(&mut self) {
        self.address = None;
        self.silence_name();
        self.stack.update_ip_addrs(|addrs| addrs.clear());
        self.stack.routes_mut().remove_default_ipv4_route();
    }

    /// In the device's own network: how many phones took an address from
    /// it (0 in another network).
    pub fn guests(&self) -> u8 {
        let seconds = u64::try_from(now().total_millis() / 1000).unwrap_or(0);
        self.server
            .as_ref()
            .map_or(0, |(_, server)| server.guests(seconds))
    }

    /// In a network with an address: answers for the device's `.local`
    /// name (`paperpaw_x4::mdns`) at `address`. Without a name, or memory
    /// for the socket, the device is only found by its address.
    fn answer_for_name(&mut self, address: [u8; 4]) {
        let Some(responder) = self
            .label
            .and_then(|label| paperpaw_x4::mdns::Responder::new(label, address))
        else {
            return;
        };
        if let Some((_, kept)) = &mut self.names {
            *kept = responder;
            return;
        }
        let Ok(handle) = self.open_udp(2, 2 * 512) else {
            return;
        };
        let Some(raw) = handle.of(self.generation) else {
            return;
        };
        let socket = self.sockets.get_mut::<udp::Socket<'_>>(raw);
        socket.close();
        if socket.bind(paperpaw_x4::mdns::PORT).is_err() {
            self.sockets.remove(raw);
            return;
        }
        let group = smoltcp::wire::Ipv4Address::from(paperpaw_x4::mdns::GROUP);
        if self.stack.join_multicast_group(group).is_err() {
            log::info!("net: the multicast group was not joined");
        }
        log::info!(
            "net: answering for {}",
            core::str::from_utf8(&responder.name()).unwrap_or("?")
        );
        self.names = Some((raw, responder));
    }

    /// The address is gone: the name is not answered for, and the socket
    /// and the group are let go.
    fn silence_name(&mut self) {
        if let Some((handle, _)) = self.names.take() {
            self.sockets.remove(handle);
            let group = smoltcp::wire::Ipv4Address::from(paperpaw_x4::mdns::GROUP);
            let _ = self.stack.leave_multicast_group(group);
        }
    }

    /// Answers the queries for the device's name, while something serves
    /// (a TCP socket listens): an idle reader on a network does not
    /// announce itself.
    fn serve_names(&mut self) {
        let Some((handle, responder)) = &mut self.names else {
            return;
        };
        let serving = self.sockets.iter().any(|(_, socket)| {
            matches!(socket, smoltcp::socket::Socket::Tcp(tcp)
                if tcp.listen_endpoint().port != 0)
        });
        let socket = self.sockets.get_mut::<udp::Socket<'_>>(*handle);
        let mut answers = Vec::new();
        while let Ok((query, meta)) = socket.recv() {
            let from = meta.endpoint;
            if serving && let Some(reply) = responder.answer(query, from.port) {
                answers.push((reply, from));
            }
        }
        let group = smoltcp::wire::IpEndpoint::new(
            IpAddress::Ipv4(smoltcp::wire::Ipv4Address::from(paperpaw_x4::mdns::GROUP)),
            paperpaw_x4::mdns::PORT,
        );
        for (reply, from) in answers {
            let (bytes, to) = match reply {
                paperpaw_x4::mdns::Reply::Group(bytes) => (bytes, group),
                paperpaw_x4::mdns::Reply::Asker(bytes) => (bytes, from),
            };
            if let Err(e) = socket.send_slice(&bytes, to) {
                log::info!("net: the name was not answered: {e:?}");
            }
        }
    }

    /// In the device's own network: answers those asking for an address.
    fn serve_addresses(&mut self) {
        let Some((handle, server)) = &mut self.server else {
            return;
        };
        let socket = self.sockets.get_mut::<udp::Socket<'_>>(*handle);
        let mut answers = Vec::new();
        while let Ok((request, _)) = socket.recv() {
            let seconds = u64::try_from(now().total_millis() / 1000).unwrap_or(0);
            if let Some(answer) = server.answer(request, seconds) {
                answers.push(answer);
            }
        }
        // To all: those asking have no address yet.
        let to = smoltcp::wire::IpEndpoint::new(
            IpAddress::Ipv4(smoltcp::wire::Ipv4Address::BROADCAST),
            paperpaw_x4::dhcp::CLIENT_PORT,
        );
        for answer in answers {
            if let Err(e) = socket.send_slice(&answer, to) {
                log::info!("net: address not given: {e:?}");
            }
        }
    }

    /// Starts finding the address of host `name` (at once if it is one);
    /// [`Net::resolved`] tells it.
    pub fn resolve(&mut self, name: &str) -> Result<Resolve, Error> {
        if let Ok(address) = name.parse::<core::net::Ipv4Addr>() {
            return Ok(Resolve::Done(IpAddress::Ipv4(address)));
        }
        let handle = self.dns.ok_or(Error::Offline)?;
        self.sockets
            .get_mut::<dns::Socket<'_>>(handle)
            .start_query(self.stack.context(), name, DnsQueryType::A)
            .map(|query| Resolve::Asking(Query::new(query, self.generation)))
            .map_err(|_| Error::Name)
    }

    /// The address asked for with [`Net::resolve`], once known; an
    /// earlier Net's lookup failed.
    pub fn resolved(&mut self, query: Query) -> Option<Result<IpAddress, Error>> {
        let Some(query) = query.of(self.generation) else {
            return Some(Err(Error::Offline));
        };
        let handle = self.dns?;
        match self
            .sockets
            .get_mut::<dns::Socket<'_>>(handle)
            .get_query_result(query)
        {
            Ok(addresses) => Some(addresses.first().copied().ok_or(Error::Name)),
            Err(dns::GetQueryResultError::Pending) => None,
            Err(dns::GetQueryResultError::Failed) => Some(Err(Error::Name)),
        }
    }

    /// Stops finding an address no longer wanted.
    pub fn forget(&mut self, query: Query) {
        if let (Some(handle), Some(query)) = (self.dns, query.of(self.generation)) {
            self.sockets
                .get_mut::<dns::Socket<'_>>(handle)
                .cancel_query(query);
        }
    }

    /// Opens a TCP connection to `to`; [`Net::socket`] then reaches it, and
    /// [`Net::release`] lets it go.
    pub fn open(&mut self, to: (IpAddress, u16), buffers: Buffers) -> Result<Handle, Error> {
        let handle = self.sockets.add(tcp_socket(buffers)?);
        let local = self.next_port();
        let connected = self.sockets.get_mut::<tcp::Socket<'_>>(handle).connect(
            self.stack.context(),
            to,
            local,
        );
        if connected.is_err() {
            self.sockets.remove(handle);
            return Err(Error::Connection);
        }
        Ok(Handle::new(handle, self.generation))
    }

    /// Waits for a connection on `port` (from any address of the local
    /// network); [`Net::socket`] reaches it, [`Net::listen_again`] makes it
    /// wait for the next once done, and [`Net::release`] lets it go.
    pub fn listen(&mut self, port: u16, buffers: Buffers) -> Result<Handle, Error> {
        let mut socket = tcp_socket(buffers)?;
        socket.listen(port).map_err(|_| Error::Connection)?;
        Ok(Handle::new(self.sockets.add(socket), self.generation))
    }

    /// Ends the connection of a socket made by [`Net::listen`] at once,
    /// and waits for the next one on `port`.
    pub fn listen_again(&mut self, handle: Handle, port: u16) {
        let Some(handle) = handle.of(self.generation) else {
            return;
        };
        self.sockets.get_mut::<tcp::Socket<'_>>(handle).abort();
        // Sends the reset before the socket listens again.
        self.stack.poll(now(), &mut self.phy, &mut self.sockets);
        let _ = self.sockets.get_mut::<tcp::Socket<'_>>(handle).listen(port);
    }

    /// Opens a UDP socket for small datagrams (SNTP's, broadcasts):
    /// `packets` of `bytes` in all, each way; let it go with
    /// [`Net::release_udp`].
    pub fn open_udp(&mut self, packets: usize, bytes: usize) -> Result<Handle, Error> {
        let buffer = || -> Result<udp::PacketBuffer<'static>, Error> {
            let mut metadata = Vec::new();
            metadata
                .try_reserve_exact(packets)
                .map_err(|_| Error::Memory)?;
            metadata.resize(packets, udp::PacketMetadata::EMPTY);
            Ok(udp::PacketBuffer::new(metadata, zeroed(bytes)?))
        };
        let mut socket = udp::Socket::new(buffer()?, buffer()?);
        let local = self.next_port();
        socket.bind(local).map_err(|_| Error::Connection)?;
        Ok(Handle::new(self.sockets.add(socket), self.generation))
    }

    /// A UDP socket of [`Net::open_udp`]; an earlier Net's sends nothing
    /// and receives nothing.
    pub fn udp(&mut self, handle: Handle) -> &mut udp::Socket<'static> {
        match handle.of(self.generation) {
            Some(handle) => self.sockets.get_mut::<udp::Socket<'_>>(handle),
            None => &mut self.gone().1,
        }
    }

    pub fn release_udp(&mut self, handle: Handle) {
        if let Some(handle) = handle.of(self.generation) {
            self.sockets.remove(handle);
        }
    }

    /// The next local port for a connection.
    fn next_port(&mut self) -> u16 {
        let port = self.port;
        self.port = if self.port >= 65000 {
            49152
        } else {
            self.port + 1
        };
        port
    }

    /// A TCP connection of [`Net::open`] or [`Net::listen`]; an earlier
    /// Net's is closed.
    pub fn socket(&mut self, handle: Handle) -> &mut tcp::Socket<'static> {
        match handle.of(self.generation) {
            Some(handle) => self.sockets.get_mut::<tcp::Socket<'_>>(handle),
            None => &mut self.gone().0,
        }
    }

    /// What an earlier Net's handles reach, made when first asked for.
    fn gone(&mut self) -> &mut (tcp::Socket<'static>, udp::Socket<'static>) {
        self.gone.get_or_insert_with(|| {
            let packets = || udp::PacketBuffer::new(Vec::new(), Vec::new());
            alloc::boxed::Box::new((
                tcp::Socket::new(
                    tcp::SocketBuffer::new(Vec::new()),
                    tcp::SocketBuffer::new(Vec::new()),
                ),
                udp::Socket::new(packets(), packets()),
            ))
        })
    }

    /// Closes a connection at once and frees it (an earlier Net's is
    /// gone already).
    pub fn release(&mut self, handle: Handle) {
        let Some(handle) = handle.of(self.generation) else {
            return;
        };
        self.sockets.get_mut::<tcp::Socket<'_>>(handle).abort();
        // Sends the reset before the socket goes.
        self.stack.poll(now(), &mut self.phy, &mut self.sockets);
        self.sockets.remove(handle);
    }
}

/// A TCP socket with `buffers`, or [`Error::Memory`].
fn tcp_socket(buffers: Buffers) -> Result<tcp::Socket<'static>, Error> {
    Ok(tcp::Socket::new(
        tcp::SocketBuffer::new(zeroed(buffers.receive)?),
        tcp::SocketBuffer::new(zeroed(buffers.send)?),
    ))
}

/// Finding a host's address.
pub enum Resolve {
    Done(IpAddress),
    Asking(Query),
}
