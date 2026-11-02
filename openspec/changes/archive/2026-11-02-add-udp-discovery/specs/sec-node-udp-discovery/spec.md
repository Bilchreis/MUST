# Spec Delta

## Purpose

Lets SECoP clients find running must SEC nodes on the network without knowing their host and TCP port, through SECoP's UDP discovery protocol (listener on port 10767, `discover` requests, `node` replies and self-announcements), compatible with Frappy nodes and clients.

## ADDED Requirements

### Requirement: Discovery listener
A running node SHALL, unless discovery is turned off, listen for UDP datagrams on port 10767 of all IPv4 interfaces from the moment its TCP port is open until it stops. It SHALL share that port so that several must nodes and Frappy nodes on one host can listen at the same time.

#### Scenario: Two nodes on one host
- **WHEN** two must nodes start in different processes on one host, and a Frappy node is already listening on UDP port 10767
- **THEN** all three listen on port 10767 and a broadcast `{"SECoP":"discover"}` gets a reply from each

#### Scenario: Two nodes in one process
- **WHEN** a process starts nodes on TCP ports 2055 and 2056
- **THEN** a broadcast `{"SECoP":"discover"}` gets one reply with `"port":2055` and one with `"port":2056`

### Requirement: Answering discover requests
The node SHALL answer each datagram that is a JSON object with `"SECoP":"discover"` by sending one reply packet to the datagram's source address and port. It SHALL ignore any other key in the request.

#### Scenario: Request with extra keys
- **WHEN** a client sends `{"SECoP":"discover","x":1}` from 192.168.1.20:50000
- **THEN** the node sends its reply packet to 192.168.1.20:50000

### Requirement: Ignored datagrams
The node SHALL silently ignore every datagram that is not a valid discover request: invalid UTF-8 or JSON, a JSON value that is not an object, a missing or different `SECoP` value (including other nodes' `node` packets), and datagrams longer than 1024 bytes. It SHALL keep listening afterwards.

#### Scenario: Another node's announcement
- **WHEN** the node receives `{"SECoP":"node","port":10767,"equipment_id":"x","firmware":"frappy","description":""}`
- **THEN** it sends nothing and still answers a later discover request

#### Scenario: Garbage
- **WHEN** the node receives the bytes `hello` and then `[1,2]`
- **THEN** it sends nothing and still answers a later discover request

### Requirement: Reply packet
A reply packet SHALL be compact JSON (no insignificant whitespace, non-ASCII characters as UTF-8) of an object with `SECoP` set to `node`, `port` set to the node's TCP port, and `equipment_id`, `firmware` and `description` set to the node's properties. When the node has no `firmware` property, `firmware` SHALL be `must-<version>`.

#### Scenario: Node bound to port 0
- **WHEN** node `HZB-cryo` with description `a cryostat` and firmware `must-0.1.0` is bound to `0.0.0.0:0` and gets TCP port 41234
- **THEN** its reply is `{"SECoP":"node","port":41234,"equipment_id":"HZB-cryo","firmware":"must-0.1.0","description":"a cryostat"}`

#### Scenario: No firmware property
- **WHEN** a node built through the C ABI without a `firmware` property is discovered, with library version 0.1.0
- **THEN** its reply has `"firmware":"must-0.1.0"`

### Requirement: Packet size limit
No discovery packet SHALL exceed 508 bytes. If it would, the node SHALL shorten `description` to the longest prefix that ends on a UTF-8 character boundary and keeps the packet within the limit. If the packet is too long even with an empty description, the node SHALL send no discovery packets at all and log a warning, and SHALL keep serving TCP.

#### Scenario: Long ASCII description
- **WHEN** a node's description is 600 ASCII characters
- **THEN** its reply is at most 508 bytes and its `description` is a prefix of the original

#### Scenario: Multi-byte characters
- **WHEN** a node's description is 400 copies of a 4-byte UTF-8 character
- **THEN** its reply is valid UTF-8 JSON of at most 508 bytes whose description holds only whole characters

#### Scenario: Equipment id too long
- **WHEN** a node's `equipment_id` is 500 characters long
- **THEN** the node starts, logs a warning that it cannot take part in discovery, and answers no discover request

### Requirement: Startup announcement
When the node starts listening, it SHALL broadcast its reply packet once to UDP port 10767 at the broadcast address of every IPv4 interface that has one, or at `255.255.255.255` if no interface has one. A failed send SHALL be logged at debug level and SHALL not stop the node or its listener.

#### Scenario: Listener sees the announcement
- **WHEN** a client listens on UDP port 10767 with the port shared and a node starts on the same network
- **THEN** the client receives the node's packet without sending a request

### Requirement: Loopback-bound nodes
A node whose TCP address is a loopback address SHALL answer only discover requests from loopback addresses and SHALL send no startup announcement.

#### Scenario: Remote request to a local-only node
- **WHEN** a node is bound to `127.0.0.1:2055` and a discover request arrives from 192.168.1.20
- **THEN** the node sends nothing

#### Scenario: Local request to a local-only node
- **WHEN** a node is bound to `127.0.0.1:2055` and a discover request arrives from 127.0.0.1
- **THEN** the node answers with `"port":2055`

### Requirement: Reply rate limit
A node SHALL send at most 20 replies in any one-second window and SHALL drop further requests in that window without replying.

#### Scenario: Request flood
- **WHEN** 1000 discover requests reach a node within one second
- **THEN** it sends at most 20 replies in that second and answers again normally once the flood stops

### Requirement: Discovery never stops a node
If the discovery socket cannot be opened, or the UDP port cannot be bound, the node SHALL log a warning naming the reason, start and serve TCP clients normally, and run without discovery.

#### Scenario: Port held by another user
- **WHEN** on Linux, a process of another user holds UDP port 10767 with port sharing and a must node starts
- **THEN** the node logs a warning, its TCP port opens, and clients can connect

### Requirement: Stopping discovery
Stopping a node SHALL close its discovery socket. Other nodes' discovery SHALL be unaffected.

#### Scenario: Stop one of two nodes
- **WHEN** nodes on TCP ports 2055 and 2056 run in one process and the first is stopped
- **THEN** a later discover request gets a reply only with `"port":2056`

### Requirement: Discovery configuration
Discovery SHALL be on by default for nodes built through the Rust API and the C ABI. The application SHALL be able to turn it off per node, and to choose another UDP port and turn off the startup announcement (for tests and special setups). The environment variable `MUST_DISCOVERY=0`, read when a node starts, SHALL turn discovery off for that node.

#### Scenario: Turned off in code
- **WHEN** a node is built with discovery turned off
- **THEN** it opens no UDP socket and answers no discover request

#### Scenario: Turned off for a C host
- **WHEN** a LabVIEW program using the C ABI is started with `MUST_DISCOVERY=0`
- **THEN** none of its nodes opens a UDP socket

#### Scenario: Another port
- **WHEN** a node is configured with discovery port 20767 and announcements off
- **THEN** it answers discover requests sent to UDP port 20767, not 10767, and broadcasts nothing at startup
