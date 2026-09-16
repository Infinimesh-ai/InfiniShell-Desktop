BUNDLE_NAME = InfiniShellDockTilePlugin.docktileplugin
OBJC_FILES = InfiniShellDockTilePlugin.m
FRAMEWORKS = -framework Cocoa -framework AppKit -framework Foundation
# InfiniShell macOS desktop releases support Apple Silicon only.
CFLAGS = -fobjc-arc -bundle -mmacosx-version-min=$(MACOSX_DEPLOYMENT_TARGET) -arch arm64
LDFLAGS = -ObjC

.PHONY: all clean

all: $(BUNDLE_NAME)

$(BUNDLE_NAME): $(OBJC_FILES)
	mkdir -p $(BUNDLE_NAME)/Contents/MacOS
	clang $(CFLAGS) $(LDFLAGS) $(FRAMEWORKS) $(OBJC_FILES) -o $(BUNDLE_NAME)/Contents/MacOS/InfiniShellDockTilePlugin
	cp Info.plist $(BUNDLE_NAME)/Contents/
	mkdir -p $(BUNDLE_NAME)/Contents/Resources
	cp Resources/* $(BUNDLE_NAME)/Contents/Resources/

clean:
	rm -rf $(BUNDLE_NAME)
