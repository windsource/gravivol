FROM gcr.io/distroless/static-debian12:nonroot

ARG TARGETARCH
COPY dist/${TARGETARCH}/gravivol /usr/local/bin/gravivol

CMD ["/usr/local/bin/gravivol"]
