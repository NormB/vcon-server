+++
title = "holds-vcons"
template = "problem.html"
[extra]
status = 409
problem_title = "The tenant holds vCons; removing them needs force"
+++

## When

`DELETE /admin/tenants/{id}` without `force=true`, and the tenant keeps at least one version, deleted ones included.

## What to do

Add `?force=true` if the tenant's vCons should go too.
