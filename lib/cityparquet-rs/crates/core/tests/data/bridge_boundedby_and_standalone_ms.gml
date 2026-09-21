<?xml version="1.0" encoding="utf-8"?>
<!-- hand-authored CityParquet test fixture (robustness).                      -->
<!-- A Bridge with NO solid that carries geometry at LoD2 twice over: once as  -->
<!-- a standalone brid:lod2MultiSurface and once inside a brid:boundedBy       -->
<!-- semantic surface. Two geometries at one LoD is not a richer model — the   -->
<!-- encoder keeps the first per object+LoD, so emitting both would silently   -->
<!-- discard whichever carried the semantics.                                  -->
<CityModel xmlns:brid="http://www.opengis.net/citygml/bridge/2.0"
           xmlns:gml="http://www.opengis.net/gml"
           xmlns="http://www.opengis.net/citygml/2.0">
	<cityObjectMember>
		<brid:Bridge gml:id="the-bridge">
			<brid:lod2MultiSurface>
				<gml:MultiSurface>
					<gml:surfaceMember>
						<gml:Polygon gml:id="face-a">
							<gml:exterior>
								<gml:LinearRing>
									<gml:posList srsDimension="3">0 0 0 10 0 0 10 10 0 0 10 0 0 0 0</gml:posList>
								</gml:LinearRing>
							</gml:exterior>
						</gml:Polygon>
					</gml:surfaceMember>
				</gml:MultiSurface>
			</brid:lod2MultiSurface>
			<brid:boundedBy>
				<brid:GroundSurface gml:id="ground">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="face-b">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList srsDimension="3">0 0 0 10 0 0 10 10 0 0 10 0 0 0 0</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:GroundSurface>
			</brid:boundedBy>
		</brid:Bridge>
	</cityObjectMember>
</CityModel>
