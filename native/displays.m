// What the menu bar and the Dock take of a display, from NSScreen: the space
// between its frame and its visible frame, at the top and at the bottom. 0 when
// no screen has that id, or when the menu bar or the Dock hides itself.
#import <AppKit/AppKit.h>

static NSScreen *lens_screen(uint32_t display) {
    for (NSScreen *screen in NSScreen.screens) {
        NSNumber *number = screen.deviceDescription[@"NSScreenNumber"];
        if (number.unsignedIntValue == display) {
            return screen;
        }
    }
    return nil;
}

double lens_menu_bar_points(uint32_t display) {
    NSScreen *screen = lens_screen(display);
    return screen ? NSMaxY(screen.frame) - NSMaxY(screen.visibleFrame) : 0;
}

double lens_dock_points(uint32_t display) {
    NSScreen *screen = lens_screen(display);
    return screen ? NSMinY(screen.visibleFrame) - NSMinY(screen.frame) : 0;
}
