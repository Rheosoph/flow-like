FROM scratch
ARG TARGETARCH
ADD rootfs/${TARGETARCH}.tar /
ENV PATH=/usr/local/bin:/usr/bin:/bin
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
USER 65532:65532
WORKDIR /var/lib/flow-like
VOLUME ["/var/lib/flow-like"]
ENTRYPOINT ["/usr/local/bin/flow-like-standalone"]
CMD ["run"]
