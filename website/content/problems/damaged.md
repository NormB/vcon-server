+++
title = "damaged"
template = "problem.html"
[extra]
status = 500
problem_title = "A stored version is damaged"
+++

## When

A stored version's data is bad: its row is kept but its file is missing, or
the file, or its wrapped key, does not decrypt and authenticate. With the
objects in an S3 bucket (on `main` and in no release yet; see
[keep vCons in an S3 bucket](@/docs/s3-storage.md)), its object is not in
the bucket, or does not decrypt and authenticate.

A read that the storage fails answers [internal](@/problems/internal.md)
instead, since it says nothing about the stored data: a disk error, a file
the server may not read, or a bucket that cannot be reached, refuses the
key, or does not answer in time.

## What to do

Tell the operator. The audit log records the request; `vcon-server check` lists every damaged version (see [checking the data](@/docs/operations.md#checking-the-data)).
