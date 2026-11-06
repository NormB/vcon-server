+++
title = "internal"
template = "problem.html"
[extra]
status = 500
problem_title = "The server failed"
+++

## When

An unexpected failure, such as a disk error. A read whose stored file the
storage fails to give answers this too, such as a file the server may not
read; the server logs `a stored version's object could not be read` with
the failure's `kind`. With the objects in an S3 bucket (on `main` and in no
release yet; see [keep vCons in an S3 bucket](@/docs/s3-storage.md)), a
store, replace or read whose object cannot be written to or read from the
bucket answers this: the service cannot be reached, refuses the key
(`AccessDenied`), or does not answer within `storage.s3.timeout`. A store
or replace that fails this way stores nothing: the version's row is never
committed. A read that fails this way says nothing about the stored data;
data that is bad answers [damaged](@/problems/damaged.md).

## What to do

Retry later and tell the operator. With an S3 bucket, the operator checks
that the service runs and answers from the server's host, and that its key
may still put and get objects (see
[when it does not start](@/docs/s3-storage.md#when-it-does-not-start) for
what each failure looks like).
