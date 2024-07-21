# Metagram

All the code for [metagram.net](https://metagram.net)

---

## Archive

This project has given me so many years of learning, but it's time for it to
finally come to an end. It turns out that a combination of [linkding] and
[Miniflux] is already way better than what I had planned to build for Firehose,
so that's what I'm using now.

This project evolved from Go microservices to a Rails modular-monolith to a
giant pile of Rust code. Deployment moved out of ephemeral containers on a
[DigitalOcean] VM onto the VM itself, and then became a set of [Fly.io]
machines. Everything was always Postgres the whole time 😉

If you find yourself wanting to ask me questions about the code you find in
here (or anything else related to this), please email me!

[DigitalOcean]: https://www.digitalocean.com
[Fly.io]: https://fly.io
[linkding]: https://github.com/sissbruecker/linkding
[Miniflux]: https://miniflux.app

---

## What is this?

Metagram is my web development playground. I someday dream of having a network
of apps that are useful, interesting, or fun to work on.

Right now, there's just [Firehose], a very opinionated personal content queue.

[Firehose]: https://metagram.net/firehose

## Join me!

I welcome contributions from anyone who finds a way to make this better.

In particular, I appreciate these things:
- Pull requests with clear motivation (tests are nice too!)
- Bug reports with reproducible setup instructions
- Ideas about things that work but can be improved
- Comments in issues and PRs to help me write better code

I will gladly help you with setup instructions (which you can use as your first
PR!) if you email me.

## License

Licensed under [GNU AGPLv3](LICENSE) or https://www.gnu.org/licenses/agpl-3.0.txt

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, shall be licensed as above, without any
additional terms or conditions.
