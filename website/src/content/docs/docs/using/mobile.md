---
title: Mobile app
description: Install Condr on your iPhone or Android phone, pair it with your computers, and check on and answer your Agents away from your desk.
---

Condr for iPhone and Android shows every Agent on the computers you pair with it, lists the ones waiting for you first, and opens any Pane so you can read and reply. The iPhone app is in public beta on TestFlight; the Android app ships as an APK in its own releases on GitHub until it reaches Google Play with the first stable release (see the [Roadmap](/docs/help/roadmap/)).

---

## Requirements

* An iPhone with iOS 26 or later, or an Android phone with Android 10 or later.
* Condr 0.1.13 or later on each computer you pair.

---

## Install

### iPhone

1. Install [TestFlight](https://apps.apple.com/app/testflight/id899247664) from the App Store.
2. On the iPhone, open the [Condr beta invitation](https://testflight.apple.com/join/6qQE7Pbx) and tap **Accept**, then **Install**.

When a new beta comes out, update the app in TestFlight. Each beta expires after 90 days, usually after a newer one is out.

### Android

1. On the phone, open the latest [Condr Companion release](https://github.com/condrdev/condr/releases?q=companion) and download `condr-companion-<version>.apk`.
2. Open the download and allow installing from your browser when Android asks.

To update, install a newer APK over it: every APK is signed with the same key, so the phone keeps its pairings. Notifications need Google Play services; a phone without them stays up to date only while the app is open.

---

## Pair a computer

1. In Condr on the computer, open **Settings › Paired devices**.
2. If **Add a phone or computer** says other devices can't connect yet, click **Allow connections**, then **Restart**. This turns on Peer-to-peer, which works from any network; restarting stops every Pane and Agent on the computer. If the phone can reach the computer's address directly, you can turn on **TCP** under **Connections** instead.
3. Click **Show pairing code**.
4. On the phone, tap **Pair with Your Computer**, then **Scan Code**, and give the computer a name.

A pairing code works once and expires after 10 minutes. On a computer without the desktop app, run `condr server invite` and tap **Paste a Link Instead** on the phone; see [Remote connections](/docs/using/remote/) for turning on Peer-to-peer or TCP from the command line.

To try the app without a computer, tap **No computer to pair? Try Demo** under **Scan Code**.

---

## Use

* **Agents**: every Agent on every paired computer, with **Needs you** at the top: the Agents that are Blocked or failed, with what they said. Tap one to open its Pane.
* **In a Pane**: the phone becomes one more window on the terminal and sizes it to the phone's screen. Clicking or typing in the Pane on the computer takes the size back, and it also returns within seconds after you leave the Pane on the phone. Type with the keyboard; the key row above it has Escape, Tab, Shift-Tab, Control, the arrows, Home, End, Page Up and Page Down. Swipe to scroll, tap to click or open a link, and long-press to copy.
* **What it doesn't do**: the phone doesn't create or close Workspaces, Tabs or Panes, or start Agents. Do that on the computer.
* **App lock**: the app asks for Face ID, Touch ID, or the passcode on an iPhone, and for your fingerprint, face, or screen lock on Android, when it starts and when you return after more than five minutes away. It is off until you turn it on in the app's Settings.

---

## Limits

* The app is connected only while it is open, and it sends no notifications yet. Push notifications for an Agent that finishes or needs you are next on the [Roadmap](/docs/help/roadmap/).
* Computers connected over SSH can't be paired with the phone. Use Peer-to-peer or TCP.
* Phones only; no iPad or tablet layout.

---

## If you lose your phone

On each computer it was paired with, open **Settings › Paired devices** and click **Revoke** next to the phone, or run `condr server revoke <fingerprint or prefix>`. A new phone pairs again as a new device.
