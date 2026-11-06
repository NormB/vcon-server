+++
title = "damaged"
template = "problem.html"
[extra]
status = 500
problem_title = "A stored version is damaged"
+++

## When

A stored version's file is missing or does not decrypt and authenticate.

With the objects in an S3 bucket (on `main` and in no release yet; see
[keep vCons in an S3 bucket](@/docs/s3-storage.md)), a read answers this
when the version's object is not in the bucket, and also whenever the object
cannot be read from it: the service cannot be reached, refuses the key, or
does not answer within `storage.s3.timeout`. The answer does not tell these
apart, so a `damaged` answer while the service is down is not proof that the
object is lost.

## What to do

Tell the operator. With an S3 bucket, the operator first checks that the
service answers from the server's host and that the key may get objects;
`vcon-server check`, run with the server stopped, then lists the versions
whose object is really missing. The audit log records the request; `vcon-server check` lists every damaged version (see [checking the data](@/docs/operations.md#checking-the-data)).
