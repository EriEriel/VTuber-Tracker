# Canary (Bun 1.4.0), not stable 1.3.x, and deliberately so: bson 7.3.1 —
# pulled in by mongoose -> mongodb — has a module-level `static {}` block
# that calls node:v8's `startupSnapshot.isBuildingSnapshot()`. Stable Bun
# throws ERR_NOT_IMPLEMENTED there, so the process dies on import before it
# ever reaches connectToDatabase(). Canary returns `false` instead.
# This also matches the canary Bun used for local dev.
#
# Switch to `oven/bun:1-slim` once 1.4 ships stable, and verify with:
#   docker run --rm oven/bun:1-slim bun -e \
#     'console.log(process.getBuiltinModule("v8").startupSnapshot.isBuildingSnapshot())'
# It should print `false`, not throw.
FROM oven/bun:canary

WORKDIR /app

# --chown matters more than it looks: COPY preserves the *source* file mode,
# and these files are 0600 on the host, so without it they land as
# 0600 root:root and the non-root `bun` user below can't read its own code
# (Bun dies with `EACCES reading /app/src/index.ts` at startup).
#
# Manifests first, separately from source: this layer only rebuilds when
# dependencies actually change, so editing src/ doesn't re-run bun install.
COPY --chown=bun:bun package.json bun.lock ./
RUN bun install --frozen-lockfile --production && chown -R bun:bun /app

COPY --chown=bun:bun tsconfig.json ./
COPY --chown=bun:bun src ./src

# The oven/bun image ships a non-root `bun` user (uid 1000). Nothing here
# needs root — the process only makes outbound calls and binds one port.
USER bun

# index.ts reads process.env.PORT and falls back to 3000. Declared here so
# `docker inspect` shows it and so it can be overridden without a rebuild.
ENV PORT=3000

# Makes assertApiTokenConfigured() fail closed: a container started without
# API_TOKEN exits immediately instead of quietly serving an open API.
ENV NODE_ENV=production

EXPOSE 3000

CMD ["bun", "run", "src/index.ts"]
