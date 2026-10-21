---
title: Privacy
description: What Condr's desktop app, mobile app, website, and services can see, and what stays on your own devices.
---

Condr has no accounts and no analytics. Neither the desktop app, the Server, nor the mobile app reports anything about you or your work to us; the only exception is the text of the notifications you allow on your phone. This page lists everything that does leave your devices.

---

## What stays on your devices

* **Your work**: terminals, Agent output, Workspaces, Git data, and files stay on the Device whose Server runs them. Other Devices see them only over connections you set up, and those are encrypted end to end. A notification carries a few details about an Agent, listed under Condr Cloud below, never terminal contents.
* **Device keys**: a computer keeps its key in the `device-key` file in its data directory. The iPhone app keeps its key in the iOS Keychain, readable only on that phone and never restored to another one from a backup. Keys never leave the device.
* **The mobile app's data**: the computers you paired, the names you gave them, and your preferences are stored on the phone.

---

## Services Condr runs

* **Peer-to-peer relay and DNS** (`relay.condr.dev`, `dns.condr.dev`): used only for Peer-to-peer connections. The relay can't read your traffic, but while it is in the path it sees which Device connects to which, when, and from which IP address. It stores nothing on disk. See [Security model](/docs/help/security/) for details.
* **Condr Cloud** (`push.condr.dev`, running on Cloudflare Workers): delivers the mobile app's notifications. When an Agent finishes, fails or needs you, your Server asks Condr Cloud to push, and Condr Cloud passes it to Apple, or to Google on Android.
  * **What it stores**: for each phone that allows notifications, its Device id, its push token, which of your Devices may push to it, and when it last registered. It keeps no notification. A registration not refreshed for 90 days is deleted, and a token is deleted as soon as Apple or Google reports it invalid.
  * **What a push shows**: Condr Cloud and Cloudflare, which runs it, Apple and Google see the Agent's name and state, the Workspace, the Agent's short message if it gave one, the terminal title, the Server's name and both Device ids. They never see terminal contents. Pushes are plain text, not end-to-end encrypted.
  * **No accounts**: requests are signed with the Device key.
* **Update check**: every five hours the desktop app reads `https://condr.dev/version.txt`, or the GitHub API if it follows the nightly channel, to find out whether a newer build exists. The request carries nothing that identifies you or your installation. Turn it off under **Settings › About**.
* **Website** (`condr.dev`): hosted on Cloudflare Pages, with no analytics and no cookies of ours. Pages load their fonts from Google Fonts. Cloudflare and Google receive your IP address as with any web request.

---

## The mobile app

* **Camera**: used only to scan the pairing code your computer shows. Nothing is saved or sent.
* **Face ID**: the app lock asks iOS, which tells Condr only whether you were recognized.
* **Local network**: used to connect directly to a computer on the same Wi-Fi.
* **Connections**: the app connects only to the computers you paired with it, to the relay and DNS above when a computer uses Peer-to-peer, and to Condr Cloud if you allow notifications. It connects only while it is open.
* **Notifications**: the app asks once, after your first pairing. If you decline, it never contacts Condr Cloud. Turning notifications off in iOS Settings stops them.
* **Crash reports and TestFlight feedback**: if you share crash data with app developers in iOS Settings, or send feedback from TestFlight, Apple passes it to us under [Apple's privacy policy](https://www.apple.com/legal/privacy/).

---

## Contact

Questions about privacy: [hello@condr.dev](mailto:hello@condr.dev). Every change to this page is in the [repository's history](https://github.com/condrdev/condr/commits/main/website/src/content/docs/docs/help/privacy.md).
