Find optimal clusterings and optimal hierarchical clusterings.


## todos
- Add more manifest information
- Add more README information, e.g. badges and links to docs, once up
- Block-quote License and Conribution sections in README?
- Figure out what KubeJS and MC version this supports, and support many
  reasonable versions of either
- Add benchmarks
- Perhaps switch to a regex engine that has compatibility with Java's
  regexes, as long as benchmarks permit this switch at least.
- Use that one method to discover what lines of code are untested. In
  particular, ensure that all the report messages are tested, and in
  the respective tests, that they are very legible.
- If selector.rs's regex fails due to taking too much time, you shouldn't
  silently default to no match, you should instead emit some information.
  Either just at debug time, but perhaps even preferrably at runtime.
  Users might care about this.
- Proper licensing of the text fixture data.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.


## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
