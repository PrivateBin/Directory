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

- `CRON`: (Optional) If set when running the app a cron cycle is performed.
  Set it to `CRON=FULL` to run a full cron (once a day).
- `GEOIP_MMDB`: path to the GeoIP database, in MaxMind format
- `ROCKET_DATABASES`: [database dict](https://api.rocket.rs/v0.4/rocket_contrib/databases/index.html#environment-variables)
  for Diesel SQLite library integration into Rocket
- `ROCKET_SECRET_KEY`: Needed in production environments, used to protect
  private cookies, generate this using `openssl rand -base64 32`

## Volumes

- `/var/directory.sqlite`: Database file, needs to be writeable
- `/var/geoip-country.mmdb`: GeoIP database, country level is sufficient

## Network ports

- `8000/tcp`: HTTP of web service

## Usage

```shell
make help
```
