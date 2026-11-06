+++
title = "internal"
template = "problem.html"
[extra]
status = 500
problem_title = "The server failed"
+++

## When

An unexpected failure, such as a disk error. With the objects in an S3
bucket (on `main` and in no release yet; see
[keep vCons in an S3 bucket](@/docs/s3-storage.md)), a store or replace
whose object cannot be written to the bucket answers this too: the service
cannot be reached, refuses the key (`AccessDenied`), or does not answer
within `storage.s3.timeout`. Nothing is stored then: the version's row is
never committed.

## What to do

Retry later and tell the operator. With an S3 bucket, the operator checks
that the service runs and answers from the server's host, and that its key
may still put objects (see
[when it does not start](@/docs/s3-storage.md#when-it-does-not-start) for
what each failure looks like).
