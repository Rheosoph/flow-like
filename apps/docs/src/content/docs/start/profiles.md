---
title: Profiles
description: Switch Between Different Profiles in Flow-Like
sidebar:
  order: 40
---

Select the Profile name in the upper-left navigation to switch Profiles, add a
Profile, or edit the active one. Profiles can be used without signing in;
signed-in Profiles can also carry account-backed settings.

![Switching Profiles and opening Profile settings from the navigation](../../../assets/SwitchAndEditProfiles.webp)

A **Profile** groups preferences that should change together, including model
assignments, interests, theme, and FlowPilot model settings:

![A screenshot of Flow-Like Desktop showing a preview of the Profile Settings page](../../../assets/ProfileSettings.webp)

For example, keep separate work and personal Profiles, or create one Profile
for lightweight local models and another for provider-backed reasoning models.
Changing the Profile changes the active configuration; it does not move or
duplicate your Apps.

On Desktop, each Profile has its own sign-in session for its configured hub.
Desktop keeps refresh credentials in encrypted native storage and renews them
without an open app window. When a login expires or is revoked, Desktop opens
the browser to sign in to that Profile again. Signing out invalidates that
Profile's running authenticated work; another Profile's login does not replace it.
Native sign-in requires HTTPS, with HTTP allowed for loopback development endpoints.

After upgrading, a previously shared browser login can migrate to one Profile.
Sign in separately to any other Profiles that used that login.

Each Profile can also have a personal Home layout. See
[Customize your Home](/start/home/) for arranging widgets and choosing between
personal, published, and bundled defaults.
