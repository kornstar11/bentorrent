# Bentorrent

## Sources:
* https://wiki.theory.org/BitTorrentSpecification#Tracker_HTTP.2FHTTPS_Protocol
* https://www.bittorrent.org/beps/bep_0003.html

## Profiling
Done with [Samply](https://github.com/mstange/samply) 

`$ cargo build --profile profiling && samply record ./target/profiling/bentorrent`