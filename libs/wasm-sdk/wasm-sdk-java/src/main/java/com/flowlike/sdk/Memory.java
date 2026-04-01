package com.flowlike.sdk;

import org.teavm.interop.Address;
import org.teavm.interop.Export;

public final class Memory {

    private static byte[] resultKeepAlive;

    private Memory() {}

    public static long packI64(int ptr, int len) {
        return ((long) ptr << 32) | (len & 0xFFFFFFFFL);
    }

    public static int unpackPtr(long packed) {
        return (int) (packed >>> 32);
    }

    public static int unpackLen(long packed) {
        return (int) (packed & 0xFFFFFFFFL);
    }

    public static int allocate(int size) {
        if (size <= 0) return 0;
        byte[] buf = new byte[size];
        return Address.ofData(buf).toInt();
    }

    public static void deallocate(int ptr, int size) {
        // TeaVM GC handles deallocation
    }

    public static long stringToWasm(String s) {
        if (s == null || s.isEmpty()) return 0;
        byte[] bytes = stringToUtf8(s);
        int ptr = Address.ofData(bytes).toInt();
        return packI64(ptr, bytes.length);
    }

    public static String wasmToString(int ptr, int len) {
        if (ptr == 0 || len <= 0) return "";
        byte[] bytes = new byte[len];
        Address src = Address.fromInt(ptr);
        for (int i = 0; i < len; i++) {
            bytes[i] = src.add(i).getByte();
        }
        return utf8ToString(bytes);
    }

    public static String unpackString(long packed) {
        if (packed == 0) return "";
        return wasmToString(unpackPtr(packed), unpackLen(packed));
    }

    public static long packResult(String json) {
        if (json == null || json.isEmpty()) return 0;
        byte[] bytes = stringToUtf8(json);
        resultKeepAlive = bytes;
        int ptr = Address.ofData(bytes).toInt();
        return packI64(ptr, bytes.length);
    }

    public static long serializeDefinition(Types.NodeDefinition def) {
        return packResult(def.toJson());
    }

    public static long serializeResult(Types.ExecutionResult result) {
        return packResult(result.toJson());
    }

    public static Types.ExecutionInput parseInput(int ptr, int len) {
        String json = wasmToString(ptr, len);
        return Json.parseExecutionInput(json);
    }

    @Export(name = "alloc")
    public static int wasmAlloc(int size) {
        return allocate(size);
    }

    @Export(name = "dealloc")
    public static void wasmDealloc(int ptr, int size) {
        deallocate(ptr, size);
    }

    @Export(name = "get_abi_version")
    public static int wasmGetAbiVersion() {
        return Types.ABI_VERSION;
    }

    static int[] stringToPtrLen(String s) {
        if (s == null || s.isEmpty()) return new int[]{0, 0};
        byte[] bytes = stringToUtf8(s);
        return new int[]{Address.ofData(bytes).toInt(), bytes.length};
    }

    private static byte[] stringToUtf8(String s) {
        int len = 0;
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c < 0x80) len++;
            else if (c < 0x800) len += 2;
            else len += 3;
        }
        byte[] bytes = new byte[len];
        int pos = 0;
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c < 0x80) {
                bytes[pos++] = (byte) c;
            } else if (c < 0x800) {
                bytes[pos++] = (byte) (0xC0 | (c >> 6));
                bytes[pos++] = (byte) (0x80 | (c & 0x3F));
            } else {
                bytes[pos++] = (byte) (0xE0 | (c >> 12));
                bytes[pos++] = (byte) (0x80 | ((c >> 6) & 0x3F));
                bytes[pos++] = (byte) (0x80 | (c & 0x3F));
            }
        }
        return bytes;
    }

    private static String utf8ToString(byte[] bytes) {
        StringBuilder sb = new StringBuilder(bytes.length);
        int i = 0;
        while (i < bytes.length) {
            int b = bytes[i] & 0xFF;
            if (b < 0x80) {
                sb.append((char) b);
                i++;
            } else if ((b & 0xE0) == 0xC0) {
                int c = ((b & 0x1F) << 6) | (bytes[i + 1] & 0x3F);
                sb.append((char) c);
                i += 2;
            } else if ((b & 0xF0) == 0xE0) {
                int c = ((b & 0x0F) << 12) | ((bytes[i + 1] & 0x3F) << 6) | (bytes[i + 2] & 0x3F);
                sb.append((char) c);
                i += 3;
            } else {
                sb.append('?');
                i++;
            }
        }
        return sb.toString();
    }
}
