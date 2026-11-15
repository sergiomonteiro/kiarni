# Native Linux amd64 package prepared by scripts/prepare_container.py.
FROM ubuntu:24.04@sha256:77eb101c607d6e3d4f5ad0c8a02c2136c689b98aba1f359e6502ba517e5c57ec
LABEL org.opencontainers.image.title="Kiarni development" \
      org.opencontainers.image.source="https://github.com/sergiomonteiro/kiarni" \
      org.opencontainers.image.licenses="MIT"
COPY package/ /usr/local/share/kiarni/
RUN cd /usr/local/share/kiarni && sha256sum -c CHECKSUMS.sha256 \
    && ln -s /usr/local/share/kiarni/kiarni /usr/local/bin/kiarni \
    && install -d -m 0700 -o 10001 -g 10001 /var/lib/kiarni
ENV KIARNI_DATA_DIR=/var/lib/kiarni/data
USER 10001:10001
WORKDIR /var/lib/kiarni
VOLUME ["/var/lib/kiarni"]
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=15s --timeout=5s --start-period=15s --retries=3 \
    CMD ["/usr/local/bin/kiarni", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/kiarni"]
CMD ["serve"]
