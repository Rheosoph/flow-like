FROM alpine:3.23
RUN apk add --no-cache postfix ca-certificates
COPY apps/backend/mail-ingress/postfix/main.cf /etc/postfix/main.cf
COPY apps/backend/mail-ingress/postfix/start.sh /usr/local/bin/start-mail
RUN chmod 755 /usr/local/bin/start-mail
EXPOSE 25
VOLUME ["/var/spool/postfix"]
ENTRYPOINT ["/usr/local/bin/start-mail"]

