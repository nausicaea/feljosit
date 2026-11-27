# Operator guide: host Feljosit for a family

This guide is for the operator of a family server (see
[ADR-0023](adr/0023-family-server.md)). The operator is one trusted family
member. The other family members only use a browser.

> [!WARNING]
> Feljosit is not designed for the open internet. Do not forward a port of your
> router to the server. For access from outside the home, use a VPN.

## Your responsibilities

1. Keep the server in the home network or in a private VPN.
2. Use TLS for each connection from a different device.
3. Use full-disk encryption on the server.
4. Make encrypted backups of the database.
5. Install each new release.
6. Add the users and set new passwords.

You can read the data of all users. All users must trust you. Tell this to each
family member before you add them.

## Requirements

- A Linux server (x86-64 or ARM64), for example a home server or a NAS.
- Docker or Podman.
- A TLS reverse proxy. This guide uses [Caddy](https://caddyserver.com/).
- A host name for the server in the home network, for example `budget.home.arpa`.

## Procedure: start Feljosit on the server

1. Verify the signature of the image (see the README, "Procedure: verify the container image").
2. Write this file `compose.yaml`. Replace `VERSION` and the host name:

   ```yaml
   services:
     feljosit:
       image: ghcr.io/nausicaea/feljosit:VERSION
       restart: unless-stopped
       environment:
         FELJOSIT_ALLOWED_HOSTS: budget.home.arpa
         FELJOSIT_SECURE_COOKIES: "true"
       ports:
         # Only the reverse proxy on the same server can connect.
         - "127.0.0.1:8765:8765"
       volumes:
         - feljosit-data:/data
   volumes:
     feljosit-data:
   ```

3. Start the container:

   ```bash
   docker compose up --detach
   ```

## Procedure: configure the TLS reverse proxy

1. Write this file `Caddyfile`:

   ```text
   budget.home.arpa {
       tls internal
       reverse_proxy 127.0.0.1:8765
   }
   ```

2. Start Caddy on the server.
3. Install the root certificate of Caddy on each device of the family. Caddy
   writes it to `pki/authorities/local/root.crt` in its data directory.
4. Open `https://budget.home.arpa` on a device. The browser must show no
   certificate warning.

The reverse proxy must send the `Host` header without a change. Caddy does this
by default. Feljosit refuses a different host name (DNS rebinding protection) and
compares the `Origin` header with the `Host` header.

If your home network has a domain with a public DNS name, Caddy can get a
certificate from Let's Encrypt with the DNS challenge. Then you do not have to
install a root certificate on the devices.

## Procedure: add a user

The browser cannot add a user behind a reverse proxy (ADR-0019).

1. Add the user. The command asks for a temporary password (minimum 12 characters):

   ```bash
   docker compose exec feljosit feljosit create-user NAME
   ```

2. Give the username and the temporary password to the family member. Do not
   send the password in a message that other persons can read.
3. Tell the family member to change the password at the first login (user guide,
   "Procedure: change your password"). The application does not force this change.

## Procedure: set a new password

Use this procedure if a user forgot the password, or if the account is locked.

1. Set a temporary password. This also unlocks the account:

   ```bash
   docker compose exec feljosit feljosit set-password NAME
   ```

2. Tell the user to change the password at the next login.

All sessions of the user close.

## Procedure: make a backup

1. Stop the container. Feljosit closes the database:

   ```bash
   docker compose stop feljosit
   ```

2. Copy the file `feljosit.sqlite3` from the volume to an encrypted backup medium.
3. Start the container again:

   ```bash
   docker compose start feljosit
   ```

## Procedure: install a new release

1. Make a backup.
2. Verify the signature of the new image.
3. Change `VERSION` in `compose.yaml`.
4. Start the new container:

   ```bash
   docker compose up --detach
   ```

The new version upgrades the database schema when it starts.

## Signs of an attack

Examine these log lines (`docker compose logs feljosit`):

- Many lines `"POST /login" 401` (failed logins), or a locked account that the user did not lock.
- Many lines with the status 400. For example, a wrong `Host` header gives this status.

The log shows the address of the reverse proxy, not of the device. To find the
device, examine the log of the reverse proxy.
