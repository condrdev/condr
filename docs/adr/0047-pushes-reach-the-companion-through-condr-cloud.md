# Pushes Reach the Companion Through Condr Cloud

Status: accepted 2026-10-21. Supersedes the roadmap's encrypted `detail` and ADR 0040's note that encrypting the payload stays the plan: pushes are plain text and the Companion has no notification extension. Amends ADR 0011 and ADR 0025, whose `authorized-clients` becomes `authorized-clients.json`; ADR 0033, whose move out of the configuration directory still moves it by its old name; ADR 0028, whose first Client hint in `Hello` is `push`; and ADR 0039, whose shared framework for an extension loses its trigger.

A Companion that is not in the foreground holds no connection (ADR 0040), so an Agent that finishes or waits for its person can reach the phone only through APNs or FCM. Their credentials cannot ship inside every user's Server, so Condr runs a service that holds them: Condr Cloud. A Server asks Condr Cloud to push; Condr Cloud forwards to Apple or Google. It is unrelated to the Relay, which carries Peer-to-peer bytes and stores nothing.

What peers do was checked first (`docs/research/push.md`). Paseo, Happy and Omnara push plain text through Expo; Superset has no mobile push. Of the relays with a published design, Web Push's VAPID and Bitwarden authenticate the caller with a short-lived token, while ntfy, Matrix's Sygnal and Home Assistant authenticate no caller and treat the push target itself as the capability. None signs the body or the path, all accept replay inside TLS, and all bound abuse with scopes and rate limits: ntfy and Bitwarden per IP, Home Assistant per recipient. Condr Cloud takes VAPID's self-signed short-lived token and Bitwarden's limits per IP.

## The service

Condr Cloud is a Spring Boot application in its own private repository, `condr-cloud`, built on Condr's existing Java service template, which it keeps whole. It answers at `https://api.condr.dev` and versions its API with Spring's API version header (`API-Version: 1`). This ADR covers only pushes; an admin backend and where it is deployed are decided later.

It has three endpoints:

- `PUT /devices/{id}` registers a phone, replacing whatever was there: `platform` (`apns` or `fcm`), `environment` (`sandbox` or `production`, APNs only), the push `token` (at most 200 bytes), and `servers`, at most 64 Device ids allowed to push to it. The Companion sends it on every launch, whenever the token changes and whenever a Device is added or removed.
- `DELETE /devices/{id}` removes the registration, sent when the Companion finds it has lost notification permission or holds no Device.
- `POST /pushes` names up to 16 recipient Device ids, a duplicate counted once, and carries one notification. It always answers 202 and never says which recipient was delivered, so a caller learns nothing about other people's phones.

A push reaches a recipient only when that phone registered and lists the caller among its `servers`; anything else is dropped silently.

## Authentication

Every request carries `Authorization: Condr <token>`. The token is a compact JWS that the caller signs with its own device key (ADR 0025): header `alg: EdDSA` with `kid` set to the caller's Device id, claims `aud` (the service's base URL), `iat`, and `exp` five minutes after `iat`. Condr Cloud issues nothing. It reads the public key straight out of `kid`, since a Device id is its public key, checks the signature, `aud` and the times with one minute of skew, and refuses an `exp` further out than five minutes. `PUT` and `DELETE /devices/{id}` also require `id` to equal `kid`. Neither the Server nor the phone gains a credential, and nobody logs in.

Since anyone can make a key, a valid token proves only that its caller holds one: it stops a caller from acting as another Device, not from becoming as many new Devices as it likes. The limits below, not the signature, bound that. A token is also a bearer credential for its five minutes, and one that leaks can be replayed with any body until it expires; it travels only over TLS and is never logged.

The scheme is not `Bearer` so the template's own JWT filter, which reads `Bearer` for its logins, never sees these requests; they pass through a separate `SecurityFilterChain`. On the Java side jjwt 0.13 and the JDK's Ed25519 verify the token with a key locator reading `kid`, adding no dependency. On the Rust side `condr-client` builds the token by hand from what it already has (`ed25519-dalek`, `base64`, `serde_json`), and both the Server and `condr-mobile` call that one function.

Failures use HTTP status codes rather than the template's `R` envelope: 400 for a malformed request, 401 for a bad or expired token, 403 for an `id` that is not `kid`, 413 past 16 KB, 429 past a rate limit.

## Storage and limits

Postgres holds two tables, migrated by Flyway: the phone registrations (Device id, platform, environment, token, last update) and their allowed Servers. A token is deleted when APNs answers 410 or FCM answers `UNREGISTERED`, but only while the registration still holds the token that push went to and, for APNs, was last updated before the time Apple gives for the token going invalid, so a phone that registered in between keeps its new token. A registration not refreshed in 90 days is purged. Condr Cloud stores no notification and logs only the path, the status, the caller's Device id and the latency, never a body, token or payload. The template's `ConsoleLogAspect` logs arguments at DEBUG and on every exception, so its pointcut excludes the push package; nothing else of the template changes.

Rate limits use Bucket4j (`bucket4j_jdk17-core`), its buckets held in memory in a size-bounded Caffeine cache. Before the token is checked, each source IP gets 10 registrations and 120 pushes a minute; keys are free, so these are what keep one machine from filling the tables with registrations, as ntfy and Bitwarden limit per IP. After it, each calling Server gets 60 pushes a minute with a burst of 20, and each phone 10 registrations a minute. A Server that gets 429 drops that push.

APNs is reached with Pushy over token authentication (a `.p8` key), FCM with the Firebase Admin SDK; their credentials come from environment variables or file paths. Condr Cloud retries APNs only on the codes Apple marks retryable. The FCM path is written but untested until Android exists (roadmap step 8).

## What a push says

A push is plain text. Its title is a localization key with the Agent's name as its argument, so the phone shows "Claude Code finished" in its own language; its body is `detail · Workspace · terminal title · Server`, the Server named by its `device_name` (ADR 0040), with empty parts left out. Two custom fields name the Server's Device id and the Pane id. Condr Cloud sets the Device id from `kid` rather than trusting the caller, and folds it with the Pane id into the collapse id, APNs' `apns-collapse-id` and FCM's `tag`, so each Pane keeps one notification, which a newer push replaces; `<Device id>:<Pane id>` fits APNs' 64 bytes. FCM carries the same fields.

APNs and FCM both refuse a payload past 4 KB, and a Workspace name has no length limit, so the Server cuts the body to 1 KB of UTF-8 at a character boundary, and Condr Cloud refuses with 400 a notification whose platform payload would still pass 4 KB. Each push expires an hour after Condr Cloud receives it (`apns-expiration`, FCM's `ttl`, whose default keeps a message four weeks): a phone that comes back later finds the Agent's current state in the app rather than a notification that may no longer be true.

Condr Cloud, Apple and Google therefore see the Agent's name and state, the Workspace, `detail`, the terminal title, the Server's name and both Device ids. They never see terminal contents. Encrypting the body was weighed and set aside: most services with end-to-end encryption send a push with no content and let the app fetch, which the Companion cannot do without a connection, and encrypting for the phone would bring a Notification Service Extension, keychain sharing between it and the app, a dedicated X25519 key and padding (`docs/research/push.md`), all for text people already see on a lock screen.

## The Server

The Server pushes on the same transitions as the desktop notification (`condr-gui/src/app/notifications.rs`): an Agent finished, failed, or needs its person, and a Blocked whose `detail` changes replaces the earlier push through the collapse id. It skips a recipient that is watching that Pane (ADR 0041) and holds back for nobody else. Input from another Client proves no one is there: an orchestrating Agent types through `condr pane` too, and a person may leave the moment they press Enter. So someone at the desktop gets the push as well. There is no `[server.push]` switch; a person who wants none turns notifications off on the phone.

The recipients are the authorized Clients that last said `push` in `Hello`, a new `bool` field and the first of ADR 0028's Client hints; an older Client sends none and gets no push. The Server records it per Client in `authorized-clients.json`, so it still knows after the phone disconnects.

The transition is found where the terminal monitor replaces a Pane's Agent snapshot, the one place that holds the previous state as well as the next; `agent_state_changed` sees only the Pane id. That thread holds the `RuntimeState` lock and has no tokio context, so it hands an owned push, recipients and text resolved, through a channel to one sender task on the Server's runtime. The task keeps only the latest push per Pane and posts with reqwest, already compiled in through iroh and declared directly at the same version, with a five-second timeout. After a timeout or a 5xx it retries once, unless a newer push for that Pane is waiting, then `warn!`s; so a late Blocked never replaces the Done that followed it. Nothing is queued past that or persisted; a push that fails is lost. `CONDR_CLOUD_URL` replaces the base URL for development and tests.

## `authorized-clients.json`

`authorized-clients` becomes `authorized-clients.json`, a JSON array of `{key, name, paired_at, last_seen, push}`, `key` written as the Device id. The `push` flag needs a column, and a fifth space-separated field after a free-text name is fragile; this also ends ADR 0025's exception that the identity files keep hex. It is JSON rather than protobuf (ADR 0028) because it is a short list that people open to see who can connect, like `config.toml`, not state only the program reads. The store rules (lock, atomic write) are unchanged, and `device-key` and `pending-invite` keep their formats. When the new file is missing and the old one exists, whichever process opens the identity store first converts it under the store lock, and deletes the old file only once the new one is written; the Server and the offline `condr server clients` and `revoke` share that path. The conversion runs after ADR 0033's move out of the configuration directory, which keeps moving the file by its old name. Both the name and the conversion are marked `compat(0.1.13)`.

## The Companion

Right after its first successful pairing, the Companion asks for notification permission with the system prompt and no page of its own, and never asks again after a refusal. It says `push` in `Hello` when it has permission and its registration with Condr Cloud succeeded; the flag is one for the whole app, not per Device. When registering changes that answer, the Companion reconnects its open connections so each Server learns at once rather than on the next connection. The token for Condr Cloud comes from `condr-mobile`; the request goes out through `URLSession`.

Tapping a notification opens its Pane (ADR 0041); if the Pane is gone, the Companion stays on that Device's Workspace list. It fetches the Server's authoritative state as it always does on returning to the foreground.

## Not chosen

A push relay in Rust: the template already has the web stack, migrations and deployment Condr Cloud needs. A stateless relay, where the phone hands each Server an opaque token and Condr Cloud stores nothing, as Paseo and Home Assistant do: it leaves no single place to stop a lost phone's pushes, and a Server would hold the push token. HPKE to the phone's key: see above. RFC 9421 HTTP message signatures, or a signature header of Condr's own: the libraries on both sides are immature, and no relay studied signs the request.

Not decided here, each with its trigger: where Condr Cloud runs, and how it learns a caller's IP behind a proxy, before it goes live; App Attest and Play Integrity, proving a registration comes from the Companion on a real phone, when registrations grow past what the limits per IP explain; an admin backend, when there is something to administer; FCM on a real device, with Android; muting one Device, when people ask; an encrypted `detail`, if what pushes carry grows past what a lock screen shows.

What changes with the implementation, not before: `SECURITY.md` and the privacy page say Condr Cloud stores Device ids, push tokens, allowed Servers and the last registration time, and what it, Apple and Google see; the website's security help and configuration reference and `docs/development-build.md` name `authorized-clients.json`.

Tests, in `condr-client`: a token the Rust side signs verifies in Condr Cloud's tests, which keep it as a fixture, and an expired, future-dated or wrong-audience token is refused. On the Server: the old `authorized-clients` converts to the JSON file and is deleted, whether the Server or `condr server clients` opens the store first and straight after ADR 0033's move from the configuration directory, and a missing `push` reads as false; `Hello.push` round-trips through protobuf and lands in the file; the push transitions, found on the monitor's own thread, match the desktop's, and a recipient watching the Pane is skipped; a body past 1 KB is cut at a character boundary; a push against a local stub at `CONDR_CLOUD_URL` carries the expected body and retries once, and a Blocked whose post times out behind a newer Done is not retried. In Condr Cloud: each endpoint's status codes and size limits, a notification whose payload would pass 4 KB among them; a push to a phone that does not list the caller is dropped, and a duplicate recipient gets one push; a 410 for a token the phone has since replaced deletes nothing; the limits per IP and per Device answer 429; nothing in the logs contains a token or payload, on success, a validation failure, a database error or an APNs error.
