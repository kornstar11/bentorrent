# Bentorrent

## Sources:
* https://wiki.theory.org/BitTorrentSpecification#Tracker_HTTP.2FHTTPS_Protocol
* https://www.bittorrent.org/beps/bep_0003.html

## Running

```
$ RUST_LOG=info cargo run -- -f ./test_data/debian-13.2.0-amd64-netinst.iso.torrent
```

## Profiling
Done with [Samply](https://github.com/mstange/samply) 

`$ cargo build --profile profiling && RUST_LOG=info samply record ./target/profiling/bentorrent`