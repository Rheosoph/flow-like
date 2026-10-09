FROM python:3.13-slim@sha256:bf44cdfcb76cd3b41e879bc058fc37ec5872002ccfde7fcb765e218cde0cd79c

WORKDIR /fixture
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
ADD --checksum=sha256:f34c7edfe92bd70a6c983374f978a5104ed1971f7fb864abaf463e4f0c7c01ad https://codeload.github.com/eclipse-tahu/tahu/tar.gz/84c63a80d87b27cbd112c20e6098fed08035f01e /tmp/tahu.tar.gz
RUN mkdir /opt/tahu && tar -xzf /tmp/tahu.tar.gz -C /opt/tahu --strip-components=1 && rm /tmp/tahu.tar.gz
ENV PYTHONPATH=/opt/tahu/python/core PYTHONDONTWRITEBYTECODE=1
COPY peer.py .
USER 65534:65534
EXPOSE 8080
HEALTHCHECK --interval=2s --timeout=5s --retries=30 CMD ["python", "peer.py", "--healthcheck"]
CMD ["python", "-u", "peer.py"]
