package app.anpod.receiver;

    public final class UiInterface {
        public final String name, address;
        public final int prefixLength;
        UiInterface(String name, String address, int prefixLength) {
            this.name = name; this.address = address; this.prefixLength = prefixLength;
        }
    }
