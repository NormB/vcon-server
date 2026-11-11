+++
title = "bad-setting"
template = "problem.html"
[extra]
status = 422
problem_title = "A setting's name or value is not one a tenant may set"
+++

## When

`PUT /tenant/settings` was sent a body that is not a JSON object of
setting names, or that holds:

- a name outside the list of settings;
- a value of the wrong kind: a masking setting takes `true` or `false`, and
  a session limit takes a whole number of seconds;
- a session limit outside its bounds: `session.idle_seconds` is 60 to 900,
  and `session.absolute_seconds` is 300 to 28800;
- limits that would leave `session.idle_seconds` above
  `session.absolute_seconds`, counting the values already set.

Nothing of a refused change is stored.

## What to do

Read the names and their values with `GET /tenant/settings`, and send only
those names. To put a name back to its default, send `null` as its value.
See [administer a tenant with a key](@/docs/tenant-administration.md).
