# Maxcos

macOS MacBook web simulator (Rust SSR) with multi-user isolation, MongoDB source of truth, Argon2 passwords, Spaces, Safari, Terminal sandbox, and production security hardening.

## Features

- Multi-user accounts with Argon2id password hashing
- MongoDB as sole source of truth (users, sessions, files, notes, settings)
- Session cookies (`HttpOnly`, `SameSite=Lax`, `Secure`)
- Rate-limited login / user-create (5/min/IP + audit lockout)
- CSRF tokens on form posts
- Security headers (CSP, X-Frame-Options DENY, nosniff)
- Terminal sandbox locked under `data/cache`
- System Settings → Admin audit log

## Run

```bash
# MongoDB required
export MONGODB_URI=mongodb://127.0.0.1:27017
export MONGODB_DB=maxcos
# optional for local HTTP without Secure cookies:
# export MAXCOS_INSECURE_COOKIES=1

cargo run
# open http://127.0.0.1:3000
```

See `.env.example` for environment variables.

## Security notes

Do not commit real `.env` files or production secrets. Disk under `data/` is a local Terminal cache only.
