<?xml version="1.0" encoding="utf-8"?>
<!-- hand-authored CityParquet test fixture (robustness).                      -->
<!-- A Bridge whose lod2Solid composes a face defined inside one of its own    -->
<!-- BridgeConstructionElement children. The child is a CityObject in its own  -->
<!-- right, so its polygons live in ITS registry — but the parent's solid      -->
<!-- still names them, and CityGML says it may: the construction element IS    -->
<!-- that face of the bridge. Registering a child's polygons as the parent's   -->
<!-- xlink targets resolves the reference and emits nothing extra.             -->
<CityModel xmlns:xlink="http://www.w3.org/1999/xlink"
           xmlns:brid="http://www.opengis.net/citygml/bridge/2.0"
           xmlns:gml="http://www.opengis.net/gml"
           xmlns="http://www.opengis.net/citygml/2.0">
	<cityObjectMember>
		<brid:Bridge gml:id="the-bridge">
			<brid:outerBridgeConstruction>
				<brid:BridgeConstructionElement gml:id="the-deck">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="deck-face">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList srsDimension="3">0 0 0 10 0 0 10 10 0 0 10 0 0 0 0</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:BridgeConstructionElement>
			</brid:outerBridgeConstruction>
			<brid:lod2Solid>
				<gml:Solid>
					<gml:exterior>
						<gml:CompositeSurface>
							<gml:surfaceMember xlink:href="#deck-face"/>
						</gml:CompositeSurface>
					</gml:exterior>
				</gml:Solid>
			</brid:lod2Solid>
		</brid:Bridge>
	</cityObjectMember>
</CityModel>
