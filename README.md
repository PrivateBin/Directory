# Directory

Rust based directory application to collect lists of federated instances of a
software.

The application is built primarily using the following libraries:

- https://rocket.rs - web framework, including a web server and multi-threaded request handler
- https://hyper.rs - http client and server
- https://tera.netlify.com - template engine
- https://diesel.rs - database ORM and query builder

## Configuration

The image supports the use of the following environment variables:

- `CRON_KEY`: Needed to trigger an /update run, prevents third parties to
  use this app to hammer the listed instances. Any string works, for example
  one generated using `openssl rand -hex 32`
- `GEOIP_MMDB`: path to the GeoIP database, in MaxMind format
- `DATABASE`: path to SQLite database file

## Volumes

- `/var/directory.sqlite`: Database file, needs to be writeable
- `/var/geoip-country.mmdb`: GeoIP database, country level is sufficient

## Network ports

- `8000/tcp`: HTTP of web service
- `8001/tcp`: HTTP of cron service

## Usage

```shell
make help
```
