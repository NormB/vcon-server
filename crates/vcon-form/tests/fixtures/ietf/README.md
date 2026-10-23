# IETF vCon examples

These examples were taken from IETF Internet-Draft
[draft-ietf-vcon-vcon-core-04](https://datatracker.ietf.org/doc/draft-ietf-vcon-vcon-core/04/),
Appendix A. Please reproduce this note if possible.

| File | Draft section | Form |
| --- | --- | --- |
| `ab_call_ext_rec.vcon` | A.4 Two Party Call vCon With Externally Referenced Recording | unsigned |
| `ab_call_ext_rec_signed.vcon` | A.6 Signed vCon | signed (JWS) |
| `ab_call_ext_rec_encrypted.vcon` | A.7 Encrypted vCon | encrypted (JWE) |

They are the unformatted versions the draft links to, downloaded unchanged
from the vCon working group's repository at commit
`02b4beb71fa4615ac1f78650a67036ce465a82c9`:
<https://github.com/ietf-wg-vcon/draft-ietf-vcon-vcon-core/tree/02b4beb71fa4615ac1f78650a67036ce465a82c9/examples>.
Apart from whitespace, each is identical to the JSON printed in the draft.

JSON in IETF documents is a Code Component under the IETF Trust Legal
Provisions, licensed under the Revised BSD License in `LICENSE`.

The phone numbers in these examples are the draft authors' illustrative
values. They are the only numbers outside the reserved test ranges that the
repository's hooks allow, and only in this directory.
