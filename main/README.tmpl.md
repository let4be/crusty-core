![crates.io](https://img.shields.io/crates/v/crusty-core.svg)
[![Dependency status](https://deps.rs/repo/github/let4be/crusty-core/status.svg)](https://deps.rs/repo/github/let4be/crusty-core)

# Crusty-core - build your own web crawler!

### Example - crawl single website, collect information about `TITLE` tags

!INCLUDECODE "examples/simple-short/src/main.rs" (rust)

If you want to get more fancy and configure some stuff or control your imports more precisely

!INCLUDECODE "examples/simple/src/main.rs" (rust)

### Install

Simply add this to your `Cargo.toml`
```
[dependencies]
crusty-core = "~0.39.0"
```

### Key capabilities

- multi-threaded && async on top of [tokio](https://github.com/tokio-rs/tokio)
- highly customizable filtering at each and every step
    - custom dns resolver with builtin IP/subnet filtering
    - status code/headers received(built-in content-type filters work at this step),
    - page downloaded(say we can decide not to parse DOM),
    - task filtering, complete control on -what- to follow and -how- to(just resolve dns, head, head+get)
- built on top of [hyper](https://github.com/hyperium/hyper) (http2 and gzip/deflate baked in)
- rich content extraction with [select](https://github.com/utkarshkukreti/select.rs)
- observable with [tracing](https://github.com/tokio-rs/tracing) and custom metrics exposed to user(stuff like html parsing duration, bytes sent/received)
- lots of options, almost everything is configurable either through options or code
- applicable both for focused and broad crawling
- scales with ease when you want to crawl millions/billions of domains
- it's fast, fast, fast!

### Development

- make sure `rustup` is installed: https://rustup.rs/

- make sure `pre-commit` is installed: https://pre-commit.com/

- make sure `markdown-pp` is installed: https://github.com/jreese/markdown-pp

- run `./go setup`

- run `./go check` to run all pre-commit hooks and ensure everything is ready to go for git

- run `./go release minor` to release a next minor version for crates.io

### Notes

Please see [examples](../examples) for more complicated usage scenarios.
This crawler is more verbose than some others, but it allows incredible customization at each and every step.

If you are interested in the area of broad web crawling there's [crusty](https://github.com/let4be/crusty), developed fully on top of `crusty-core` that tries to tackle on some challenges of broad web crawling
