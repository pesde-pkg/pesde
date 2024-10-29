---
title: Policies
description: Policies for the pesde registry
---

The following policies apply to the [official public pesde registry](https://registry.pesde.daimond113.com)
and its related services, such as the index repository or websites.
They may not apply to other registries. By using the pesde registry, you agree
to these policies.

If anything is unclear, please [contact us](#contact-us), and we will be happy
to help.

## Contact Us

You can contact us at [pesde@daimond113.com](mailto:pesde@daimond113.com). In
case of a security issue, please prefix the subject with `[SECURITY]`.

## Permitted content

The pesde registry is a place for Luau-related packages. This includes:

- Libraries
- Frameworks
- Tools

The following content is forbidden:

- Malicious, vulnerable code
- Illegal, harmful content
- Miscellaneous files (doesn't include configuration files, documentation, etc.)

pesde is not responsible for the content of packages, the scope owner is. It
is the responsibility of the scope owner to ensure that the content of their
packages is compliant with the permitted content policy.

If you believe a package is breaking these requirements, please [contact us](#contact-us).

## Package removal

pesde does not support removing packages for reasons such as abandonment. A
package may only be removed for the following reasons:

- The package is breaking the permitted content policy
- The package contains security vulnerabilities
- The package must be removed for legal reasons (e.g. DMCA takedown)

In case a secret has been published to the registry, it must be invalidated.
If you believe a package should be removed, please [contact us](#contact-us).
We will review your request and take action if necessary.

If we find that a package is breaking the permitted content policy, we will
exercise our right to remove it from the registry without notice.

pesde reserves the right to remove any package from the registry at any time for
any or no reason, without notice.

## Package ownership

Packages are owned by scopes. Scope ownership is determined by the first person
to publish a package to the scope. The owner of the scope may send a pull request
to the index repository adding team members' user IDs to the scope's `scope.toml`
file to give them access to the scope, however at least one package must be
published to the scope before this can be done. The owner may also remove team
members from the scope.

A scope's true owner's ID must appear first in the `owners` field of the scope's
`scope.toml` file. Ownership may be transferred by the current owner sending a
pull request to the index repository, and the new owner confirming the transfer.

Only the owner may add or remove team members from the scope.

pesde reserves the right to override scope ownership in the case of a dispute,
such as if the original owner is unresponsive or multiple parties claim ownership.

## Scope squatting

Scope squatting is the act of creating a scope with the intent of preventing
others from using it, without any intention of using it yourself. This is
forbidden and can result in the removal (release) of the scope and its packages
from the registry without notice.

If you believe a scope is being squatted, please [contact us](#contact-us).
We will review your request and take action if necessary.

## API Usage

The pesde registry has an API for querying, downloading, and publishing packages.
Only non-malicious use is permitted. Malicious uses include:

- **Service Degradation**: this includes sending an excessive amount of requests
  to the registry in order to degrade the service
- **Exploitation**: this includes trying to break the security of the registry
  in order to gain unauthorized access
- **Harmful content**: this includes publishing harmful (non-law compliant,
  purposefully insecure) content
