# Change Log

# 0.14.0
- switch to rust 1.78 to support diesel_migrations 2.2.0
- bump libraries

# 0.13.0
- fix Mozilla Observatory API v2, request is now POST, response behaved slightly
  differently, rescan not necessary nor supported anymore
- bump libraries

# 0.12.0
- switch to Mozilla Observatory API v2
- bump libraries

# 0.11.0
- adding version- and template based CSP checks #89
- addressed typos in the about page
- bump libraries

# 0.10.8
- bump libraries

# 0.10.7
- HTTP(S) client now uses hyper 1.1
- switch to rust 1.70 to drop lazy_static in favor of OnceLock
- bump libraries

# 0.10.6
- bump libraries

# 0.10.5
- bump libraries

# 0.10.4
- drop dns-lookup library in favor of stdlib usage

# 0.10.3
- bump libraries

# 0.10.2
- bump libraries

# 0.10.1
- bump libraries

# 0.10.0
- add a random redirect endpoint
- reverting enable_all_versions() use, it doesn't seem to enable HTTP 1.x

# 0.9.6
- switch to rust 1.65 and to make use of it's build time strip
- strip index.php suffix from URL, if present
- bump libraries & bootstrap css
- cleaned up docker image build, switching to a still maintained build image

# 0.9.5
- add check for content length returned by Mozilla observatory
- scan at most 1024 lines
- use read buffer wrapped iterator to protect from reading overly long lines
- bump libraries

# 0.9.4
- add unit test for check page
- fix deadlock upon evicting expired negative lookup cache
- bump libraries

# 0.9.3
- handle unwrapped errors, instead of panicking
- clarify use of casts, replacing `clone()` with `to_owned()` and `into()` over `String::from()` or `to_string()`, when types can be inferred
- add negative lookup cache, to prevent unnecessary lookups, which could be abused to cause load on queried instances
- bump libraries

# 0.9.2
- version bump unit tests
- bump libraries

# 0.9.1
- removed "script-src resource:" Content-Security-Policy, previously required for PDF preview in FireFox

# 0.9.0
- added Content-Security-Policy header rating

# 0.8.0
- adding a "check instance" form, to retrieve a detailed report on an instance, without adding it
- upgrade to bootstrap CSS 5.1.3
- fix navbar toggle in mobile display
- fix unit test that stopped working in 0.7.0
- tweak dark mode link color for improved readability

# 0.7.2
- cleaning up the request code
- switch to rust 1.58.0 and make use of it's captured identifiers
- use diesel DSL over format string SQL statements
- cache compiled regular expressions and formatted user agent string

# 0.7.1
- re-implementing support for internationalized domain names (IDN), lost in the hyper upgrade at 0.7.0
- switching to rust edition 2021

# 0.7.0
- upgrade to rocket 0.5.0-rc.1, bump libraries, update rustc

# 0.6.1
- implement additional filters for the JSON api

# 0.6.0
- JSON api for retrieving top instances, randomized for load balancing

# 0.5.2
- hard 15s timeout on client connections
- separating model and view logic of country flag

## 0.5.1
- add country name for mouse over on country flags
- increase number of displayed instances (we now have over 100 instances)
- share HTTP(S) client instance across threads
- bump libraries

## 0.5.0
- instances with disabled port 80 now get a checkmark for "HTTPS enforced"
- documented how to validate blocking the bot

## 0.4.9
- added example for caddy configuration
- bump libraries, update rustc

## 0.4.8
- fix immediate delete in full cron

## 0.4.7
- default to browsers dark mode setting, but persist the users choice #17
- delete instances, if cron detects they are no longer PrivateBin instances
- bump libraries

## 0.4.6
- avoiding unwraps, preventing threads to panic on observatory errors

## 0.4.5
- set 5 second write timeout on all HTTP(S) connections

## 0.4.4
- adding some timing diagnostics to the cron task output
- set 5 second read timeout on all HTTP(S) connections

## 0.4.3
- run threads in parallel as intended, by collecting the lazy iterators m(

## 0.4.2
- handle a number of edge cases in the URL parsing that could lead to duplicate
  entries for the same instance (i.e. URLs ending in //, or with GET parameters
  or hashes)
- bump libraries

## 0.4.1

- delete instances, if cron detects robots.txt change, asking for removal #15
- bump libraries to fix pear bug occurring in newer nightly rust compilers

## 0.4.0

- cron is now triggered by executing binary with environment variable CRON=POLL
  or CRON=FULL set, not via http call on separate port - obsoletes cron key

## 0.3.5

- fix handling internationalized URLs #14

## 0.3.4

- move as much work into threads as possible, database writes have to remain
  single threaded with SQLite

## 0.3.3

- some per instance checks can run in parallel threads as well

## 0.3.2

- multiplexing cron checks into threads

## 0.3.1

- dual-bind is unneccessary, binding to all IPv6 interfaces to supports IPv4

## 0.3.0

- support binding to IPv4 & IPv6 at the same time, using multi-threading
- separate cron service off to port 8001

## 0.2.7

- strip URL of query as well as trailing slash, fixes #12

## 0.2.6

- CSS fixes
- extending the "About" page
- applied cargo fmt styling

## 0.2.5

- added Mozilla Observatory ratings #1

## 0.2.4

- split cron into uptime check and full check #10

## 0.2.3

- added deletion of failing instances #9
- added change log
- added GitHub workflow to run tests and clippy on push
- applied clippy code quality suggestions

## 0.2.2

- fixing caching behaviour

## 0.2.1

- fade out instances with lower uptime #8
- implement robots.txt support #3
- fixing dark mode
- fixed sort, giving uptime higher priority over URL

## 0.2.0

- added display of uptime column to list
- embedded database migrations into application
- added uptime checks in additional table #8
- added cron hook for instance update #7
- added about page #4

## 0.1.3

- added dark mode switch
- fixing MacOS checkmark color font issues

## 0.1.2

- fixing docker image autobuild

## 0.1.1

- setting up docker image autobuild
- added fork-me-ribbon

## 0.1.0

- Adding and listing instances from SQLite database
- GeoIP check for country flag emoji
- checks HTTPS, redirection from HTTP, version and attachment support
- 5 minute in memory cache
- bootstrap 4.4 CSS design
