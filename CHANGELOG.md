# Change log

## 0.4.1

- delete instances, if cron detects robots.txt change, asking for removal #15
- bump libraries to fix pear bug occurring in newer nightly rust compilers

## 0.4.0

- cron is now triggered by executing binary with environment variable CRON=POLL
  or CRON=FULL set, not via http call on separate port - obsoletes cron key

## 0.3.5

- fix handling internationalized URLs #14

## 0.3.4

- move as much work into threads as possible, database writes have to remain single threaded with SQLite

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