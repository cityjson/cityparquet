# citydb-tool is a Java CLI; run it from a container so the host needs no
# specific JRE and the version is pinned into the benchmark manifest.
FROM docker.io/library/eclipse-temurin:21-jre

ARG CITYDB_TOOL_VERSION=1.3.2
ENV CITYDB_TOOL_VERSION=${CITYDB_TOOL_VERSION}

RUN apt-get update \
 && apt-get install -y --no-install-recommends curl unzip postgresql-client \
 && rm -rf /var/lib/apt/lists/*

RUN curl -sSfLo /tmp/citydb-tool.zip \
      "https://github.com/3dcitydb/citydb-tool/releases/download/v${CITYDB_TOOL_VERSION}/citydb-tool-${CITYDB_TOOL_VERSION}.zip" \
 && unzip -q /tmp/citydb-tool.zip -d /opt \
 && mv /opt/citydb-tool-${CITYDB_TOOL_VERSION} /opt/citydb-tool \
 && rm /tmp/citydb-tool.zip \
 && chmod +x /opt/citydb-tool/citydb

ENV PATH="/opt/citydb-tool:${PATH}"
WORKDIR /work
ENTRYPOINT ["citydb"]
