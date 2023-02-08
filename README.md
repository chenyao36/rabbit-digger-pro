# rabbit-digger-pro

> A picture shows a rabbit digging a wall.

[![codecov][codecov-badge]][codecov-url]
[![MIT licensed][mit-badge]][mit-url]
[![Build Status][actions-badge]][actions-url]

[codecov-badge]: https://codecov.io/gh/rabbit-digger/rabbit-digger-pro/branch/main/graph/badge.svg?token=VM9N0IGMWE
[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[actions-badge]: https://github.com/rabbit-digger/rabbit-digger-pro/workflows/Build/badge.svg

[codecov-url]: https://codecov.io/gh/rabbit-digger/rabbit-digger-pro
[mit-url]: https://github.com/rabbit-digger/rabbit-digger-pro/blob/master/LICENSE
[actions-url]: https://github.com/rabbit-digger/rabbit-digger-pro/actions?query=workflow%3ABuild+branch%3Amain

All-in-one proxy written in Rust.

## Features

* Hot reloading: Apply changes without restart the program.
* Flexible configuration: proxies can be nested at will, supporting TCP and UDP.
* JSON Schema generation: no documentation needed, write configuration directly from code completion.

### Supported Protocol

* Shadowsocks
* Trojan
* HTTP
* Socks5
* obfs(http_simple)

### Supported Server Protocol

* Socks5
* HTTP
* http+socks5 on the same port
* Shadowsocks

## crates

* rd-derive

Used to conveniently define the Config structure.

* rd-std

Some basic net and server, such as rule, HTTP and Socks5.

* rd-interface

Interface defines of rabbit-digger's plugin.

## Credits

* [shadowsocks-rust](https://github1s.com/shadowsocks/shadowsocks-rust)
* [smoltcp](https://github.com/smoltcp-rs/smoltcp)
