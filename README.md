Flag                          Reason
-c copy                       No re-encode for aligned segments
-fps_mode passthrough         Prevents FFmpeg from forcing 30fps
-avoid_negative_ts make_zero  Fixes DTS discontinuities at clip joins
-map_metadata 1               Copies global metadata from original source
-map 0                        Includes all streams (video+audio+subtitles)
-ignore_unknown               Passes through unrecognised data streams
-movflags +faststart          Moves MP4 moov atom to front for streaming
-bframes=0:keyint=1            IDR-only re-encode for frame-accurate GOP head
