"""Independent OPC UA server for the industrial adapter integration tests."""

import asyncio
from datetime import datetime, timedelta, timezone
import ipaddress
import logging
from pathlib import Path
import socket
import sys

from asyncua import Client, Server, ua
from asyncua.crypto.permission_rules import User, UserRole
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID


PKI = Path("/fixture-pki")
SERVER_URI = "urn:flow-like:test:asyncua"
ENDPOINT = "opc.tcp://0.0.0.0:4840/flow-like/"


def certificate(name, application_uri):
    # Test credentials exist only for the lifetime of this container.
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, name)])
    now = datetime.now(timezone.utc)
    cert = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(subject)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - timedelta(days=1))
        .not_valid_after(now + timedelta(days=7))
        .add_extension(
            x509.SubjectAlternativeName(
                [
                    x509.UniformResourceIdentifier(application_uri),
                    x509.DNSName("localhost"),
                    x509.DNSName(socket.gethostname()),
                    x509.IPAddress(ipaddress.ip_address("127.0.0.1")),
                ]
            ),
            critical=False,
        )
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=True,
                content_commitment=True,
                key_encipherment=True,
                data_encipherment=True,
                key_agreement=False,
                key_cert_sign=False,
                crl_sign=False,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(
            x509.ExtendedKeyUsage(
                [ExtendedKeyUsageOID.SERVER_AUTH, ExtendedKeyUsageOID.CLIENT_AUTH]
            ),
            critical=False,
        )
        .sign(key, hashes.SHA256())
    )
    (PKI / f"{name}.der").write_bytes(cert.public_bytes(serialization.Encoding.DER))
    private_key = PKI / f"{name}.pem"
    private_key.write_bytes(
        key.private_bytes(
            serialization.Encoding.PEM,
            serialization.PrivateFormat.PKCS8,
            serialization.NoEncryption(),
        )
    )
    private_key.chmod(0o600)


class FixtureUsers:
    def get_user(self, iserver, username=None, password=None, certificate=None):
        if username is None or (
            username == "flow-like-test" and password == "fixture-password"
        ):
            return User(role=UserRole.User)
        return None


async def populate(server):
    namespace = await server.register_namespace("urn:flow-like:test:values")
    assert namespace == 2
    scalars = [
        ("Boolean", False, ua.VariantType.Boolean),
        ("Byte", 0, ua.VariantType.Byte),
        ("SByte", 0, ua.VariantType.SByte),
        ("Int16", 0, ua.VariantType.Int16),
        ("UInt16", 0, ua.VariantType.UInt16),
        ("Int32", 0, ua.VariantType.Int32),
        ("UInt32", 0, ua.VariantType.UInt32),
        ("Int64", 0, ua.VariantType.Int64),
        ("UInt64", 0, ua.VariantType.UInt64),
        ("Float", 0.0, ua.VariantType.Float),
        ("Double", 0.0, ua.VariantType.Double),
        ("String", "fixture", ua.VariantType.String),
    ]
    for group in ("Anonymous", "Secure"):
        parent = await server.nodes.objects.add_object(
            ua.NodeId(group, namespace), group
        )
        for name, initial, variant_type in scalars:
            variable = await parent.add_variable(
                ua.NodeId(f"{group}.{name}", namespace),
                name,
                ua.Variant(initial, variant_type),
            )
            await variable.set_writable()
        await parent.add_variable(
            ua.NodeId(f"{group}.ReadOnly", namespace),
            "ReadOnly",
            ua.Variant(42, ua.VariantType.Int32),
        )


async def serve():
    PKI.mkdir(parents=True, exist_ok=True)
    certificate("server", SERVER_URI)
    certificate("client", "urn:flow-like:inspection")
    server = Server(user_manager=FixtureUsers())
    await server.init()
    server.set_endpoint(ENDPOINT)
    server.set_server_name("Flow-Like asyncua integration fixture")
    await server.set_application_uri(SERVER_URI)
    await server.load_certificate(PKI / "server.der")
    await server.load_private_key(PKI / "server.pem")
    server.set_security_policy(
        [
            ua.SecurityPolicyType.NoSecurity,
            ua.SecurityPolicyType.Basic256Sha256_SignAndEncrypt,
        ]
    )
    server.set_identity_tokens([ua.AnonymousIdentityToken, ua.UserNameIdentityToken])
    await populate(server)
    async with server:
        await asyncio.Event().wait()


async def healthcheck():
    async with Client("opc.tcp://127.0.0.1:4840/flow-like/", timeout=3) as client:
        assert await client.get_node("ns=2;s=Anonymous.ReadOnly").read_value() == 42


if __name__ == "__main__":
    logging.basicConfig(level=logging.WARNING)
    asyncio.run(healthcheck() if "--healthcheck" in sys.argv else serve())
