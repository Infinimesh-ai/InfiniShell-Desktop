#import <Cocoa/Cocoa.h>
#import <Foundation/Foundation.h>

@interface InfiniShellDockTilePlugIn : NSObject <NSDockTilePlugIn>
{
    id iconChangedObserver;
    id defaultsObserver;
}

@property(strong) id iconChangedObserver;
@property(strong) id defaultsObserver;
@end
