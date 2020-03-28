# Directory

Rust based directory application to collect lists of federated instances of a
software.

## Configuration

The image supports the use of the following environment variables:

- `GEOIP_MMDB`: path to the GeoIP database, in MaxMind format
- `ROCKET_DATABASES`: [database dict](https://api.rocket.rs/v0.4/rocket_contrib/databases/index.html#environment-variables)
  for Diesel SQLite library integration into Rocket

## Volumes

- `/var/directory.sqlite`: Database file, needs to be writeable
- `/var/geoip-country.mmdb`: GeoIP database, country level is sufficient

## Network ports

- `8000/tcp`: HTTP

## Usage

```shell
make help
```
