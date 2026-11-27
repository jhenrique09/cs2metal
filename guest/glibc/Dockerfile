# The Steam Runtime 3 SDK, pinned as for the fixtures, plus the two tools a glibc
# build needs that the SDK lacks.
FROM registry.gitlab.steamos.cloud/steamrt/sniper/sdk@sha256:bb30053d3cd19d35c76ae69b2790dbaa1430448006f8240cb6d7e723f2d1940e
RUN apt-get update -qq && apt-get install -y -qq gawk texinfo >/dev/null
