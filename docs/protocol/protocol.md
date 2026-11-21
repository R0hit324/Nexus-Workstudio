# Nexus Wire Protocol

Version: **1** (`nexus-core::version::PROTOCOL_VERSION`)

## Framing and serialization

Every message is serialized with `postcard` and sent inside a length-prefixed
frame:

```text
+--------+---------------+
|  len   |   payload     |
| u32 BE |  postcard     |
+--------+---------------+
```

- Length is the payload byte count, big-endian (`u32`).
- Maximum accepted payload: 16 MiB (`nexus_protocol::frame::MAX_FRAME_LENGTH`).
  Peers must reject larger frames instead of buffering them.
- Type: `Message` (see `nexus_protocol::message`).

See `docs/decisions/ADR-003-serialization-format.md`.

## Connection lifecycle

```text
Client                                   Server
  |  ClientHello{name, version, caps}      |
  |--------------------------------------->|
  |  ServerHello{name, version, caps}      |
  |<---------------------------------------|
  |  Authenticate{username}                |
  |--------------------------------------->|
  |  AuthenticationResult{session}         |
  |<---------------------------------------|
  |  ... operational messages ...          |
  |  Heartbeat{at} <-> HeartbeatAck{at}    |
  |  Disconnect{reason}                    |
  |--------------------------------------->|
```

- The client must speak the same protocol version as the server, else it must
  disconnect.
- Capabilities are advertised in both hellos; the intersection is the
  negotiated feature set.
- The server applies an idle deadline per session. A client that stops sending
  heartbeats is disconnected.

## Message catalog (Part 1)

| Message | Direction | Purpose |
|---------|-----------|---------|
| `ClientHello` | C→S | handshake open |
| `ServerHello` | S→C | handshake reply |
| `Authenticate` / `AuthenticationResult` | C→S / S→C | session establishment |
| `CreateCube` / `CubeCreated` | C→S / S→C | Cube creation |
| `ListCubes` / `CubeList` | C→S / S→C | visible Cube enumeration |
| `JoinCube` / `CubeJoined` | C→S / S→C | join a Cube |
| `LeaveCube` / `CubeLeft` | C→S / S→C | leave a Cube |
| `Event` / `EventBatch` | C→S (S→C fan-out in Part 4) | event delivery |
| `Ack` | S→C | generic acknowledgement |
| `Error` | S→C | protocol/domain error |
| `Heartbeat` / `HeartbeatAck` | C→S / S→C | keepalive |
| `Disconnect` | C→S | clean shutdown |

Requests carry a `RequestId`; responses echo it so clients can correlate
replies even while receiving asynchronous events.

## Transport abstraction

Domain code never touches sockets directly. `nexus_protocol::transport::Transport`
abstracts the connection (`send`, `receive`, `close`); `TcpTransport`
implements it over TCP with the framing above. This keeps the wire
transport-replaceable (see ADR-002).