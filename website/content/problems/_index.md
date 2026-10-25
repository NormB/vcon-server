+++
title = "Problem types"
description = "Every refusal vcon-server answers with, as an RFC 9457 problem document whose type is one of these pages."
sort_by = "title"
template = "problems.html"
page_template = "problem.html"
+++

Every refusal is an `application/problem+json` document with `type`, `title`
and `status`:

```json
{
  "type": "https://vcon-server.com/problems/conflict",
  "title": "Another vCon holds the uuid",
  "status": 409
}
```

Act on `type` (or `status`), never on `title`'s wording. No problem repeats
any part of the request, so a problem can be logged anywhere.
