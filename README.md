# HID Server

This server assists in using a single device, such as a laptop, as an input
device for multiple other devices, including phones, tablets, and computers.

## Building

This project uses Cargo. Run `cargo build` to build the HID server.

## Installation

Install the binary into `/usr/bin/hid-server`. Install the systemd unit
into `/usr/lib/systemd/system/hid-server.service`.

## Usage

When run, the HID server creates a command and event socket, either in the
current directory, or in the directory at $RUNTIME_DIRECTORY, usually
/run/hid-server.

By default, the server remains down and needs to be commanded, through the
command socket, to advertise to clients.

### Commands

A client can send commands through a seqpacket socket to put the server up and
take it down. Commands should be serialized in the following format.

```
+-----------------+------------------+
| OPCODE (1 byte) |  DATA (n bytes)  |
+-----------------+------------------+
```

| Opcode | Operation | Description |
|--------|-----------|-------------|
| 0x01   | UP        | DATA should contain an SDP record, in XML, as required by BlueZ. |
| 0x02   | DOWN      | DATA is ignored. |

### Events

Events consist of an event code and a segment of data.

```
+---------------+------------------+
| CODE (1 byte) |  DATA (n bytes)  |
+---------------+------------------+
```

Event codes

| Code | Event | Description |
|------|-------|-------------|
| 0x01 | LAGGED | DATA contains an integer `k`. This event indicates that `k` messages were skipped since the socket was last queried. |
| 0x02 | CONTROL_LISTENING | A peer has connected to the control channel. DATA contains the peer's Bluetooth address. |
| 0x03 | INTERRUPT_LISTENING | A peer has connected to the interrupt channel. DATA contains the peer's Bluetooth address. |
| 0x04 | DISCONNECTED | A peer has disconnected. DATA contains the peer's Bluetooth address. |

### Connections

Each connected device is represented by a pair of sockets, one named "control"
using the seqpacket protocol, the other named "interrupt" using the datagram
protocol. For example, a connection with a device with address 01:23:45:AB:CD:EF
would create sockets in

```
01_23_45_AB_CD_EF/control
01_23_45_AB_CD_EF/interrupt
```


