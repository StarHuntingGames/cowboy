# Copyright (C) 2026 StarHuntingGames
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.

ARG RUST_VERSION=1.93
FROM rust:${RUST_VERSION}-bookworm AS builder

WORKDIR /app
COPY backend /app

ARG SERVICE
RUN cargo build --release -p ${SERVICE}

FROM debian:bookworm-slim AS runtime

ARG SERVICE
ENV SERVICE=${SERVICE}

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && if [ "${SERVICE}" = "bot-service" ]; then apt-get install -y --no-install-recommends python3 python3-pip; fi \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY backend/bot-service/python /app/bot-agent
RUN if [ "${SERVICE}" = "bot-service" ]; then \
      python3 -m pip install --break-system-packages --no-cache-dir -r /app/bot-agent/requirements.txt; \
    fi

COPY --from=builder /app/target/release/${SERVICE} /usr/local/bin/${SERVICE}

ENTRYPOINT ["/bin/sh", "-lc", "exec /usr/local/bin/${SERVICE}"]
